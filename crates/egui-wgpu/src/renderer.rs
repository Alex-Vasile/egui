use core::{num::NonZeroU64, ops::Range};
use std::borrow::Cow;
use std::sync::Arc;

use ahash::{HashMap, HashSet};
use bytemuck::Zeroable as _;
use epaint::mutex::Mutex;
use epaint::{PaintCallbackInfo, Primitive, Vertex, emath::NumExt as _};

use wgpu::util::DeviceExt as _;

// Only implements Send + Sync on wasm32 in order to allow storing wgpu resources on the type map.
#[cfg(not(all(
    target_arch = "wasm32",
    not(feature = "fragile-send-sync-non-atomic-wasm"),
)))]
/// You can use this for storage when implementing [`CallbackTrait`].
pub type CallbackResources = type_map::concurrent::TypeMap;
#[cfg(all(
    target_arch = "wasm32",
    not(feature = "fragile-send-sync-non-atomic-wasm"),
))]
/// You can use this for storage when implementing [`CallbackTrait`].
pub type CallbackResources = type_map::TypeMap;

/// You can use this to do custom [`wgpu`] rendering in an egui app.
///
/// Implement [`CallbackTrait`] and call [`Callback::new_paint_callback`].
///
/// This can be turned into a [`epaint::PaintCallback`] and [`epaint::Shape`].
pub struct Callback(Box<dyn CallbackTrait>);

impl Callback {
    /// Creates a new [`epaint::PaintCallback`] from a callback trait instance.
    pub fn new_paint_callback(
        rect: epaint::emath::Rect,
        callback: impl CallbackTrait + 'static,
    ) -> epaint::PaintCallback {
        epaint::PaintCallback {
            rect,
            callback: std::sync::Arc::new(Self(Box::new(callback))),
        }
    }
}

/// A callback trait that can be used to compose an [`epaint::PaintCallback`] via [`Callback`]
/// for custom WGPU rendering.
///
/// Callbacks in [`Renderer`] are done in three steps:
/// * [`CallbackTrait::prepare`]: called for all registered callbacks before the main egui render pass.
/// * [`CallbackTrait::finish_prepare`]: called for all registered callbacks after all callbacks finished calling prepare.
/// * [`CallbackTrait::paint`]: called for all registered callbacks during the main egui render pass.
///
/// Each callback has access to an instance of [`CallbackResources`] that is stored in the [`Renderer`].
/// This can be used to store wgpu resources that need to be accessed during the [`CallbackTrait::paint`] step.
///
/// The callbacks implementing [`CallbackTrait`] itself must always be Send + Sync, but resources stored in
/// [`Renderer::callback_resources`] are not required to implement Send + Sync when building for wasm.
/// (this is because wgpu stores references to the JS heap in most of its resources which can not be shared with other threads).
///
///
/// # Command submission
///
/// ## Command Encoder
///
/// The passed-in [`wgpu::CommandEncoder`] is egui's and can be used directly to register
/// wgpu commands for simple use cases.
/// This allows reusing the same [`wgpu::CommandEncoder`] for all callbacks and egui
/// rendering itself.
///
/// ## Command Buffers
///
/// For more complicated use cases, one can also return a list of arbitrary
/// [`wgpu::CommandBuffer`]s and have complete control over how they get created and fed.
/// In particular, this gives an opportunity to parallelize command registration and
/// prevents a faulty callback from poisoning the main wgpu pipeline.
///
/// When using eframe, the main egui command buffer, as well as all user-defined
/// command buffers returned by this function, are guaranteed to all be submitted
/// at once in a single call.
///
/// Command Buffers returned by [`CallbackTrait::finish_prepare`] will always be issued *after*
/// those returned by [`CallbackTrait::prepare`].
/// Order within command buffers returned by [`CallbackTrait::prepare`] is dependent
/// on the order the respective [`epaint::Shape::Callback`]s were submitted in.
///
/// # Example
///
/// See the [`custom3d_wgpu`](https://github.com/emilk/egui/blob/main/crates/egui_demo_app/src/apps/custom3d_wgpu.rs) demo source for a detailed usage example.
pub trait CallbackTrait: Send + Sync {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _target_format: wgpu::TextureFormat,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        _callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        Vec::new()
    }

    /// Called after all [`CallbackTrait::prepare`] calls are done.
    fn finish_prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _egui_encoder: &mut wgpu::CommandEncoder,
        _callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        Vec::new()
    }

    /// Called after all [`CallbackTrait::finish_prepare`] calls are done.
    ///
    /// It is given access to the [`wgpu::RenderPass`] so that it can issue draw commands
    /// into the same [`wgpu::RenderPass`] that is used for all other egui elements.
    ///
    /// `target_format` is the format of the color attachment the render pass draws
    /// into — the same value [`CallbackTrait::prepare`] received. Pipelines must be
    /// created for (and keyed by) this format: it can differ between viewports and
    /// change over a viewport's lifetime, so it cannot be known up front.
    fn paint(
        &self,
        info: PaintCallbackInfo,
        target_format: wgpu::TextureFormat,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    );
}

/// Information about the screen used for rendering.
pub struct ScreenDescriptor {
    /// Size of the window in physical pixels.
    pub size_in_pixels: [u32; 2],

    /// High-DPI scale factor (pixels per point).
    pub pixels_per_point: f32,
}

impl ScreenDescriptor {
    /// size in "logical" points
    fn screen_size_in_points(&self) -> [f32; 2] {
        [
            self.size_in_pixels[0] as f32 / self.pixels_per_point,
            self.size_in_pixels[1] as f32 / self.pixels_per_point,
        ]
    }
}

/// Uniform buffer used when rendering.
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct UniformBuffer {
    screen_size_in_points: [f32; 2],
    dithering: u32,

    /// 1 to do manual filtering for more predictable kittest snapshot images.
    ///
    /// See also <https://github.com/emilk/egui/issues/5295>.
    predictable_texture_filtering: u32,

    /// The SDR-white scale applied by the linear-decode entry point only:
    /// `sdr_white_nits / 80` so egui's 1.0 stays the SDR reference white on
    /// scRGB surfaces, where 1.0 is pinned to 80 nits (plan §7). Gamma-encoded
    /// entry points never read it (their surfaces interpret 1.0 natively).
    /// Must match `Locals` in `egui.wgsl`.
    ///
    /// Field order matters: the pad sits before the scale so the trailing
    /// `f32` lands at offset 28 and the struct at 32 bytes — WGSL's uniform
    /// address space needs the 16-byte-multiple size and forbids the
    /// array/vec3 shapes you'd otherwise reach for.
    _padding: [u32; 3],
    sdr_white_scale: f32,
}

/// Pins the Rust side of the Rust↔WGSL `Locals` agreement (`egui.wgsl`): the
/// struct must stay at exactly the 32 bytes the WGSL layout produces, on
/// every machine — including those where the adapter-gated GPU tests (the
/// only other place the agreement is checked, by naga) skip.
const _: () = assert!(
    core::mem::size_of::<UniformBuffer>() == 32,
    "UniformBuffer must stay at the 32 bytes the WGSL `Locals` layout produces (16-byte \
     uniform alignment: pad before the trailing scale); any drift desyncs the uniform"
);

/// The scRGB / sRGB standard's SDR reference white: 80 nits (IEC 61966-2-1).
///
/// scRGB is scene-referred: on an `ExtendedSrgbLinear` surface a stored 1.0
/// means 80 nits, fixed — unlike macOS EDR's display-referred 1.0 (the
/// display's current SDR white, moving with the brightness slider). Scaling
/// egui output by `sdr_white_nits / 80` (plan §7) keeps the §4 contract —
/// egui's 1.0 = SDR reference white — on both platforms, so chrome matches
/// surrounding SDR apps and `sdr_white_nits` tracking the user's slider
/// brightens egui with it. Windows' SDR-content slider never touches
/// advanced-color output, so without the scaling egui would render at the
/// fixed 80-nit anchor regardless of the slider.
pub(crate) const SDR_REFERENCE_WHITE_NITS: f32 = 80.0;

/// The uniform value for the linear path: `nits / 80`, or `1.0` when the
/// display reports nothing (macOS, web, …) so the standard 80-nit anchoring
/// holds — egui's 1.0 = 80 nits (plan §7's default-when-unknown).
pub(crate) fn sdr_white_scale(sdr_white_nits: Option<f32>) -> f32 {
    sdr_white_nits.map_or(1.0, |nits| nits / SDR_REFERENCE_WHITE_NITS)
}

struct SlicedBuffer {
    buffer: wgpu::Buffer,
    slices: Vec<Range<usize>>,
    capacity: wgpu::BufferAddress,
}

pub struct Texture {
    /// The texture may be None if the `TextureId` is just a handle to a user-provided bind-group.
    pub texture: Option<wgpu::Texture>,

    /// Bindgroup for the texture + sampler.
    pub bind_group: wgpu::BindGroup,

    /// Options describing the sampler used in the bind group. This may be None if the `TextureId`
    /// is just a handle to a user-provided bind-group.
    pub options: Option<epaint::textures::TextureOptions>,
}

/// Ways to configure [`Renderer`] during creation.
#[derive(Clone, Copy, Debug)]
pub struct RendererOptions {
    /// Set the level of the multisampling anti-aliasing (MSAA).
    ///
    /// Must be a power-of-two. Higher = more smooth 3D.
    ///
    /// A value of `0` or `1` turns it off (default).
    ///
    /// `egui` already performs anti-aliasing via "feathering"
    /// (controlled by [`egui::epaint::TessellationOptions`]),
    /// but if you are embedding 3D in egui you may want to turn on multisampling.
    pub msaa_samples: u32,

    /// What format to use for the depth and stencil buffers,
    /// e.g. [`wgpu::TextureFormat::Depth32FloatStencil8`].
    ///
    /// egui doesn't need depth/stencil, so the default value is `None` (no depth or stancil buffers).
    pub depth_stencil_format: Option<wgpu::TextureFormat>,

    /// Controls whether to apply dithering to minimize banding artifacts.
    ///
    /// Dithering assumes an 8-bit sRGB target; float targets never dither —
    /// their pipelines are built from a shader module with the dither
    /// application compiled out, regardless of this knob (a float target has
    /// no 8-bit quantizer for the noise to hide in; plan §3).
    ///
    /// On an 8-bit target the noise applies to any input value that lies
    /// between two 8bit values in "gamma space". This means that only inputs
    /// from texture interpolation and vertex colors should be affected in
    /// practice.
    ///
    /// Defaults to true.
    pub dithering: bool,

    /// Perform texture filtering in software?
    ///
    /// This is useful when you want predictable rendering across
    /// different hardware, e.g. for kittest snapshots.
    ///
    /// Default is `false`.
    ///
    /// See also <https://github.com/emilk/egui/issues/5295>.
    pub predictable_texture_filtering: bool,
}

impl RendererOptions {
    /// Set options that produce the most predicatable output.
    ///
    /// Useful for image snapshot tests.
    pub const PREDICTABLE: Self = Self {
        msaa_samples: 1,
        depth_stencil_format: None,
        dithering: false,
        predictable_texture_filtering: true,
    };
}

impl Default for RendererOptions {
    fn default() -> Self {
        Self {
            msaa_samples: 0,
            depth_stencil_format: None,
            dithering: true,
            predictable_texture_filtering: false,
        }
    }
}

/// Renderer for a egui based GUI.
pub struct Renderer {
    /// Render pipelines keyed by the render-pass target's format *and* its
    /// derived [`OutputEncoding`] (plan §3): the encoding decides the
    /// fragment entry point, so one format can legitimately host two
    /// pipelines — an fp16 target reaches both encodings across viewports
    /// (`ExtendedDisplayP3` on macOS, `ExtendedSrgbLinear` on Windows/DX12).
    /// Created lazily on first use of each key, in [`Self::update_buffers`].
    /// Only the target's format and derived encoding bake a pipeline: a
    /// color-space change to a space with the same encoding reconfigures the
    /// surface and reuses the pipeline.
    pipelines: Mutex<HashMap<(wgpu::TextureFormat, OutputEncoding), Arc<wgpu::RenderPipeline>>>,

    /// The egui shader module, from which all pipelines for 8-bit targets are
    /// built (dithering compiled in).
    shader_module: wgpu::ShaderModule,

    /// The egui shader module for float targets (`Rgba16Float`), with the
    /// dither application compiled out regardless of
    /// [`RendererOptions::dithering`] — a float target has no 8-bit quantizer
    /// for the noise to hide in (plan §3). See [`Self::shader_source`].
    shader_module_float_target: wgpu::ShaderModule,

    /// The pipeline layout, shared by all pipelines in the cache.
    pipeline_layout: wgpu::PipelineLayout,

    index_buffer: SlicedBuffer,
    vertex_buffer: SlicedBuffer,

    uniform_buffer: wgpu::Buffer,
    previous_uniform_buffer_content: UniformBuffer,
    uniform_bind_group: wgpu::BindGroup,
    texture_bind_group_layout: wgpu::BindGroupLayout,

    /// Uniform buffers each holding a single `u32` of texture flags
    /// (see [`texture_flags`]), indexed by that flag value.
    /// Read by the shader when `predictable_texture_filtering` is on.
    texture_flag_buffers: [wgpu::Buffer; NUM_TEXTURE_FLAGS],

    /// Map of egui texture IDs to textures and their associated bindgroups (texture view +
    /// sampler). The texture may be None if the `TextureId` is just a handle to a user-provided
    /// sampler.
    textures: HashMap<epaint::TextureId, Texture>,
    next_user_texture_id: u64,
    samplers: HashMap<epaint::textures::TextureOptions, wgpu::Sampler>,

    /// The color space of the surface this renderer currently
    /// presents to, as granted by the surface decision. Set by the painter
    /// (winit / web) each frame, under its renderer lock, before the
    /// per-frame [`Self::update_texture`] calls; `None` (the default) means
    /// "unknown / no surface", which pairs content with today's SDR-sRGB
    /// semantics.
    ///
    /// This is the other half of the content↔surface upload pairing table:
    /// the pairing function in this file is the table, this is the surface
    /// row it looks up. It also feeds the derived pipeline
    /// selection — [`OutputEncoding::of`] turns it plus the target format
    /// into the fragment entry point (plan §3). Never branch on it anywhere
    /// else (plan §5 design rule: concrete color spaces are named only in the
    /// pairing tables).
    active_surface_color_space: Option<wgpu::SurfaceColorSpace>,

    /// The active surface display's SDR white level in nits, as reported by
    /// the platform (`DisplayLuminance.sdr_white_nits` — Windows/DXGI only,
    /// moving with the brightness slider). Set by the painter each frame next
    /// to the color space; `None` (the default — macOS, web, headless) keeps
    /// the standard 80-nit anchoring: the linear entry point scales output by
    /// `sdr_white_nits / 80`, so `None` means scale 1.0 (plan §7). Gamma
    /// encoded targets ignore it entirely.
    sdr_white_nits: Option<f32>,

    /// Texture ids whose upload was already rejected-and-warned about — a
    /// pairing rejection or a content-kind/format mismatch logs once per
    /// texture id (egui's warn-once convention for content the surface or
    /// texture cannot take). Cleared on [`Self::free_texture`] so a freed
    /// id can warn again.
    upload_rejections_warned: HashSet<epaint::TextureId>,

    options: RendererOptions,

    /// Storage for resources shared with all invocations of [`CallbackTrait`]'s methods.
    ///
    /// See also [`CallbackTrait`].
    pub callback_resources: CallbackResources,
}

impl Renderer {
    /// The WGSL source for one shader module: the shared transfer functions
    /// and entry points, prefixed by the compile-time dither gate.
    ///
    /// Two modules are built in [`Self::new`]: `DITHER_CAPABLE = true` for
    /// 8-bit targets and `false` for float targets. The gate is declared here
    /// — not in `egui.wgsl` — because the two variants must differ in exactly
    /// this one constant and WGSL has no way to inject it into the file.
    /// Keeping every other line identical (the same `include_str!` calls
    /// concatenated in the same order) is what keeps one source of truth for
    /// the shader math; the float module is the 8-bit module with dithering
    /// compiled out (plan §3).
    fn shader_source(dither_capable: bool) -> String {
        format!(
            "const DITHER_CAPABLE: bool = {dither_capable};\n{}\n{}",
            include_str!("egui_transfer.wgsl"),
            include_str!("egui.wgsl")
        )
    }

    /// Creates a renderer for a egui UI.
    ///
    /// Render pipelines are created lazily, keyed by the format of the
    /// render-pass target, the first time [`Self::update_buffers`] sees a
    /// format. Pass the actual target format of each render pass to
    /// [`Self::render`] and [`Self::update_buffers`].
    pub fn new(device: &wgpu::Device, options: RendererOptions) -> Self {
        profiling::function_scope!();

        let shader_for = |dither_capable| {
            wgpu::ShaderModuleDescriptor {
                label: Some(if dither_capable {
                    "egui"
                } else {
                    "egui (float target, dither off)"
                }),
                // The module is the shared transfer functions (`egui_transfer.wgsl`)
                // concatenated with the entry points (`egui.wgsl`), behind the
                // one-line `DITHER_CAPABLE` prologue; the CPU↔GPU round-trip test
                // concatenates the same helpers with its own entry points, so both
                // exercise the same WGSL source. The allocation is twice per
                // `Renderer::new` (the two variants), never per frame. Both
                // variants have identical line numbering — a WGSL error reports
                // the same line whichever module it came from.
                //
                // Two maintainer traps:
                // - WGSL error messages report line numbers against the
                //   CONCATENATED source: subtract `egui_transfer.wgsl`'s line
                //   count, the join newline, and the one-line dither-gate
                //   prologue to land on `egui.wgsl`.
                // - Nothing validates this concatenation at build time: `cargo
                //   check`, clippy and the wasm32 CI job never parse WGSL. The
                //   first validation of any edit to either file is this call, on
                //   a machine with a real wgpu adapter.
                source: wgpu::ShaderSource::Wgsl(Cow::Owned(Self::shader_source(dither_capable))),
            }
        };
        let module = {
            profiling::scope!("create_shader_module");
            device.create_shader_module(shader_for(true))
        };
        let module_float_target = {
            profiling::scope!("create_shader_module_float_target");
            device.create_shader_module(shader_for(false))
        };

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("egui_uniform_buffer"),
            contents: bytemuck::cast_slice(&[UniformBuffer {
                screen_size_in_points: [0.0, 0.0],
                dithering: u32::from(options.dithering),
                predictable_texture_filtering: u32::from(options.predictable_texture_filtering),
                _padding: [0; 3],
                // Overwritten from the first `update_buffers` on; 1.0 is the
                // no-report default (the standard 80-nit anchor).
                sdr_white_scale: 1.0,
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let uniform_bind_group_layout = {
            profiling::scope!("create_bind_group_layout");
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("egui_uniform_bind_group_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            core::mem::size_of::<UniformBuffer>() as _
                        ),
                        ty: wgpu::BufferBindingType::Uniform,
                    },
                    count: None,
                }],
            })
        };

        let uniform_bind_group = {
            profiling::scope!("create_bind_group");
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("egui_uniform_bind_group"),
                layout: &uniform_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniform_buffer,
                        offset: 0,
                        size: None,
                    }),
                }],
            })
        };

        let texture_bind_group_layout = {
            profiling::scope!("create_bind_group_layout");
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("egui_texture_bind_group_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(
                                (core::mem::size_of::<u32>() * 4) as _,
                            ),
                            ty: wgpu::BufferBindingType::Uniform,
                        },
                        count: None,
                    },
                ],
            })
        };

        let texture_flag_buffers = core::array::from_fn::<_, NUM_TEXTURE_FLAGS, _>(|flag| {
            let flag = flag as u32;
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("egui_texture_flags_{flag}")),
                contents: bytemuck::bytes_of(&[flag, 0, 0, 0]),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("egui_pipeline_layout"),
            bind_group_layouts: &[
                Some(&uniform_bind_group_layout),
                Some(&texture_bind_group_layout),
            ],
            immediate_size: 0,
        });

        const VERTEX_BUFFER_START_CAPACITY: wgpu::BufferAddress =
            (core::mem::size_of::<Vertex>() * 1024) as _;
        const INDEX_BUFFER_START_CAPACITY: wgpu::BufferAddress =
            (core::mem::size_of::<u32>() * 1024 * 3) as _;

        Self {
            pipelines: Mutex::new(HashMap::default()),
            shader_module: module,
            shader_module_float_target: module_float_target,
            pipeline_layout,
            vertex_buffer: SlicedBuffer {
                buffer: create_vertex_buffer(device, VERTEX_BUFFER_START_CAPACITY),
                slices: Vec::with_capacity(64),
                capacity: VERTEX_BUFFER_START_CAPACITY,
            },
            index_buffer: SlicedBuffer {
                buffer: create_index_buffer(device, INDEX_BUFFER_START_CAPACITY),
                slices: Vec::with_capacity(64),
                capacity: INDEX_BUFFER_START_CAPACITY,
            },
            uniform_buffer,
            // Buffers on wgpu are zero initialized, so this is indeed its current state!
            previous_uniform_buffer_content: UniformBuffer::zeroed(),
            uniform_bind_group,
            texture_bind_group_layout,
            texture_flag_buffers,
            textures: HashMap::default(),
            next_user_texture_id: 0,
            samplers: HashMap::default(),
            active_surface_color_space: None,
            sdr_white_nits: None,
            upload_rejections_warned: HashSet::default(),
            options,
            callback_resources: CallbackResources::default(),
        }
    }

    /// Creates (if needed) the render pipeline for `target_format` under the
    /// active surface's derived [`OutputEncoding`].
    ///
    /// Exactly one pipeline is cached per (format, encoding) key; the cache is
    /// shared by all viewports, so viewports rendering at different formats —
    /// or at one format with both encodings — coexist.
    ///
    /// The pipeline is compiled outside the lock: pipeline creation is expensive
    /// and must not stall a concurrent `render` reading the cache. A racing
    /// double compile is benign — the first pipeline inserted wins.
    fn ensure_pipeline(&self, device: &wgpu::Device, target_format: wgpu::TextureFormat) {
        let encoding = self.active_output_encoding(target_format);
        if self
            .pipelines
            .lock()
            .contains_key(&(target_format, encoding))
        {
            return;
        }

        let module = if is_float_target(target_format) {
            &self.shader_module_float_target
        } else {
            &self.shader_module
        };
        let pipeline = Arc::new(create_pipeline(
            device,
            module,
            &self.pipeline_layout,
            &self.options,
            target_format,
            encoding,
        ));
        self.pipelines
            .lock()
            .entry((target_format, encoding))
            .or_insert(pipeline);
    }

    /// Executes the egui renderer onto an existing wgpu renderpass.
    ///
    /// Note that the lifetime of `render_pass` is `'static` which requires a call to [`wgpu::RenderPass::forget_lifetime`].
    /// This allows users to pass resources that live outside of the callback resources to the render pass.
    /// The render pass internally keeps all referenced resources alive as long as necessary.
    /// The only consequence of `forget_lifetime` is that any operation on the parent encoder will cause a runtime error
    /// instead of a compile time error.
    ///
    /// `target_format` is the format of the color attachment the render pass draws into.
    /// It selects the cached pipeline, so [`Self::update_buffers`] must have been
    /// called with it at some point before (viewports rendering at different
    /// formats can interleave their `update_buffers` calls freely).
    ///
    /// # Panic
    /// Panics if [`Renderer::update_buffers`] has never been called with this
    /// `target_format` under the currently active surface color space (the
    /// cache key is (format, derived encoding))!
    pub fn render(
        &self,
        render_pass: &mut wgpu::RenderPass<'static>,
        paint_jobs: &[epaint::ClippedPrimitive],
        screen_descriptor: &ScreenDescriptor,
        target_format: wgpu::TextureFormat,
    ) {
        profiling::function_scope!();

        let pipeline = {
            let pipelines = self.pipelines.lock();
            pipelines
                .get(&(target_format, self.active_output_encoding(target_format)))
                .cloned()
                .expect(
                    "You must call .update_buffers() with the same target format before .render()",
                )
        };

        let pixels_per_point = screen_descriptor.pixels_per_point;
        let size_in_pixels = screen_descriptor.size_in_pixels;

        // Whether or not we need to reset the render pass because a paint callback has just
        // run.
        let mut needs_reset = true;

        let mut index_buffer_slices = self.index_buffer.slices.iter();
        let mut vertex_buffer_slices = self.vertex_buffer.slices.iter();

        for epaint::ClippedPrimitive {
            clip_rect,
            primitive,
        } in paint_jobs
        {
            if needs_reset {
                render_pass.set_viewport(
                    0.0,
                    0.0,
                    size_in_pixels[0] as f32,
                    size_in_pixels[1] as f32,
                    0.0,
                    1.0,
                );
                render_pass.set_pipeline(&pipeline);
                render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                needs_reset = false;
            }

            {
                let rect = ScissorRect::new(clip_rect, pixels_per_point, size_in_pixels);

                if rect.width == 0 || rect.height == 0 {
                    // Skip rendering zero-sized clip areas.
                    if let Primitive::Mesh(_) = primitive {
                        // If this is a mesh, we need to advance the index and vertex buffer iterators:
                        index_buffer_slices
                            .next()
                            .expect("You must call .update_buffers() before .render()");
                        vertex_buffer_slices
                            .next()
                            .expect("You must call .update_buffers() before .render()");
                    }
                    continue;
                }

                render_pass.set_scissor_rect(rect.x, rect.y, rect.width, rect.height);
            }

            match primitive {
                Primitive::Mesh(mesh) => {
                    let index_buffer_slice = index_buffer_slices
                        .next()
                        .expect("You must call .update_buffers() before .render()");
                    let vertex_buffer_slice = vertex_buffer_slices
                        .next()
                        .expect("You must call .update_buffers() before .render()");

                    if let Some(Texture { bind_group, .. }) = self.textures.get(&mesh.texture_id) {
                        render_pass.set_bind_group(1, bind_group, &[]);
                        render_pass.set_index_buffer(
                            self.index_buffer.buffer.slice(
                                index_buffer_slice.start as u64..index_buffer_slice.end as u64,
                            ),
                            wgpu::IndexFormat::Uint32,
                        );
                        render_pass.set_vertex_buffer(
                            0,
                            self.vertex_buffer.buffer.slice(
                                vertex_buffer_slice.start as u64..vertex_buffer_slice.end as u64,
                            ),
                        );
                        render_pass.draw_indexed(0..mesh.indices.len() as u32, 0, 0..1);
                    } else {
                        log::warn!("Missing texture: {:?}", mesh.texture_id);
                    }
                }
                Primitive::Callback(callback) => {
                    let Some(cbfn) = callback.callback.downcast_ref::<Callback>() else {
                        // We already warned in the `prepare` callback
                        continue;
                    };

                    let info = PaintCallbackInfo {
                        viewport: callback.rect,
                        clip_rect: *clip_rect,
                        pixels_per_point,
                        screen_size_px: size_in_pixels,
                    };

                    let viewport_px = info.viewport_in_pixels();
                    if viewport_px.width_px > 0 && viewport_px.height_px > 0 {
                        profiling::scope!("callback");

                        needs_reset = true;

                        // We're setting a default viewport for the render pass as a
                        // courtesy for the user, so that they don't have to think about
                        // it in the simple case where they just want to fill the whole
                        // paint area.
                        //
                        // The user still has the possibility of setting their own custom
                        // viewport during the paint callback, effectively overriding this
                        // one.
                        render_pass.set_viewport(
                            viewport_px.left_px as f32,
                            viewport_px.top_px as f32,
                            viewport_px.width_px as f32,
                            viewport_px.height_px as f32,
                            0.0,
                            1.0,
                        );

                        cbfn.0
                            .paint(info, target_format, render_pass, &self.callback_resources);
                    }
                }
            }
        }

        render_pass.set_scissor_rect(0, 0, size_in_pixels[0], size_in_pixels[1]);
    }

    /// Tell the renderer which color space the presenting surface was
    /// granted, so the content↔surface upload pairing can gate
    /// [`Self::update_texture`].
    ///
    /// Called by the painter (winit / web) each frame, under its renderer
    /// write lock, before the per-frame `update_texture` calls, with the
    /// viewport's granted [`SurfaceTarget::color_space`]. `None` (the
    /// default) pairs content with today's SDR-sRGB semantics, so
    /// app-driven/headless `update_texture` calls (kittest, custom
    /// integrations) keep working unchanged.
    ///
    /// The value is trusted, not verified: misreporting it re-arms exactly
    /// the misinterpretation the pairing gate exists to prevent (a wrong
    /// space "honors" tags it doesn't, instead of loudly rejecting) — always
    /// pass the viewport's actual granted space. It also *persists*: every
    /// `update_texture` is evaluated against the value from the most recent
    /// call, so an app driving uploads directly is paired against the last
    /// painted viewport's space.
    pub fn set_active_surface_color_space(&mut self, color_space: Option<wgpu::SurfaceColorSpace>) {
        self.active_surface_color_space = color_space;
    }

    /// Tells the renderer the active surface display's SDR white level in
    /// nits, so the linear-decode entry point can scale output by
    /// `sdr_white_nits / 80` and egui's 1.0 stays the SDR reference white on
    /// scRGB surfaces (plan §7).
    ///
    /// Called by the painter (winit / web) each frame, under its renderer
    /// write lock, on the same paint path that already polls
    /// `display_hdr_info` for the headroom — no extra OS query. winit reads
    /// [`wgpu::DisplayLuminance::sdr_white_nits`] (Windows/DXGI only, tracks
    /// the user's brightness slider); web has no reporting and passes `None`.
    /// `None` means the standard 80-nit anchoring holds (scale 1.0), and the
    /// value only ever reaches the float (scRGB) target class — every other
    /// target pins the uniform to 1.0 at the write site.
    pub fn set_sdr_white_nits(&mut self, sdr_white_nits: Option<f32>) {
        self.sdr_white_nits = sdr_white_nits;
    }

    /// The [`OutputEncoding`] the render pass into `target_format` must use,
    /// derived from the active surface color space (unknown → `Srgb`
    /// semantics, the app-driven/headless default). Derived on every use,
    /// never stored (plan §3).
    fn active_output_encoding(&self, target_format: wgpu::TextureFormat) -> OutputEncoding {
        OutputEncoding::of(
            target_format,
            self.active_surface_color_space
                .unwrap_or(wgpu::SurfaceColorSpace::Srgb),
        )
    }

    /// Should be called before [`Self::render`].
    pub fn update_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: epaint::TextureId,
        image_delta: &epaint::ImageDelta,
    ) {
        profiling::function_scope!();

        // The content↔surface upload pairing lives here, inside
        // `update_texture` (plan §4), WITHOUT changing its signature:
        // rejection is not an error — the upload is not performed, a warning
        // logs once per texture id, and egui's standard placeholder renders.
        // A `Result`-returning upload is future work.
        let upload: UploadData<'_> = match &image_delta.image {
            epaint::ImageData::Color(image) => UploadData {
                bytes: bytemuck::cast_slice(image.pixels()),
                bytes_per_pixel: 4,
                format: wgpu::TextureFormat::Rgba8Unorm,
            },
            epaint::ImageData::Float(image) => {
                match pair_float_content_with_surface(
                    image.primaries,
                    self.active_surface_color_space,
                ) {
                    Ok(()) => UploadData {
                        bytes: bytemuck::cast_slice(image.pixels()),
                        bytes_per_pixel: 8,
                        format: wgpu::TextureFormat::Rgba16Float,
                    },
                    Err(rejection) => {
                        if self.upload_rejections_warned.insert(id) {
                            // The warning must state the honest reason for the
                            // ACTUAL inputs: every rejection message carries
                            // the evaluated (content primaries, active
                            // surface color space) pair.
                            log::warn!(
                                "egui texture {id:?}: {rejection} (content primaries: {:?}, \
                                 active surface color space: {:?})",
                                image.primaries,
                                self.active_surface_color_space,
                            );
                        }
                        return;
                    }
                }
            }
        };

        let width = image_delta.image.width() as u32;
        let height = image_delta.image.height() as u32;

        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        // `bytes_per_row` for the float path is padded to wgpu's 256-byte
        // alignment (plan §4) so `write_texture` needs no internal staging
        // pass; rows are repacked accordingly. The u8 color path keeps
        // today's exact layout.
        let raw_bytes_per_row = upload.bytes_per_pixel * width;
        let bytes_per_row = if upload.format == wgpu::TextureFormat::Rgba16Float {
            raw_bytes_per_row.div_ceil(WGPU_BYTES_PER_ROW_ALIGNMENT) * WGPU_BYTES_PER_ROW_ALIGNMENT
        } else {
            raw_bytes_per_row
        };
        let padded_bytes: Cow<'_, [u8]> = if bytes_per_row == raw_bytes_per_row {
            Cow::Borrowed(upload.bytes)
        } else {
            let mut padded = vec![0u8; bytes_per_row as usize * height as usize];
            for row in 0..height {
                let src = (row * raw_bytes_per_row) as usize;
                let dst = (row * bytes_per_row) as usize;
                padded[dst..dst + raw_bytes_per_row as usize]
                    .copy_from_slice(&upload.bytes[src..src + raw_bytes_per_row as usize]);
            }
            Cow::Owned(padded)
        };

        let queue_write_data_to_texture = |texture, origin| {
            profiling::scope!("write_texture");
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin,
                    aspect: wgpu::TextureAspect::All,
                },
                &padded_bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
                size,
            );
        };

        // Use same label for all resources associated with this texture id (no point in retyping the type)
        let label_str = format!("egui_texid_{id:?}");
        let label = Some(label_str.as_str());

        let (texture, origin, bind_group) = if let Some(pos) = image_delta.pos {
            // Content-kind consistency: a patch's texel layout must match
            // the format the managed texture was created with. A Color patch
            // onto a float texture fails wgpu's size validation (a confusing
            // device error); a float patch onto an 8-bit texture would write
            // an 8-byte-per-texel payload into a 4-byte-texel texture
            // (silent corruption). Bail like the pairing gate: warn once per
            // texture id, skip the patch — the previous content stays.
            let existing_format = self
                .textures
                .get(&id)
                .and_then(|t| t.texture.as_ref().map(wgpu::Texture::format));
            if existing_format.is_some_and(|existing| existing != upload.format) {
                if self.upload_rejections_warned.insert(id) {
                    log::warn!(
                        "egui texture {id:?}: skipping partial update at {pos:?}: the patch's \
                         texels are {:?} but the texture was created as {:?} — re-upload the \
                         whole texture to change its kind",
                        upload.format,
                        existing_format,
                    );
                }
                return;
            }

            let Texture {
                texture,
                bind_group,
                options,
            } = self
                .textures
                .remove(&id)
                .expect("Tried to update a texture that has not been allocated yet.");
            let texture = texture.expect("Tried to update user texture.");
            let options = options.expect("Tried to update user texture.");
            let origin = wgpu::Origin3d {
                x: pos[0] as u32,
                y: pos[1] as u32,
                z: 0,
            };

            (
                texture,
                origin,
                // If the TextureOptions are the same as the previous ones, we can reuse the bind group. Otherwise we
                // have to recreate it.
                if image_delta.options == options {
                    Some(bind_group)
                } else {
                    None
                },
            )
        } else {
            // allocate a new texture
            let texture = {
                profiling::scope!("create_texture");
                device.create_texture(&wgpu::TextureDescriptor {
                    label,
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: upload.format,
                    // `COPY_SRC` enables the upload-verification readbacks
                    // (plan §8 row 2b: "read the GPU texture back").
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_DST
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[upload.format],
                })
            };
            let origin = wgpu::Origin3d::ZERO;
            (texture, origin, None)
        };

        let bind_group = bind_group.unwrap_or_else(|| {
            let nearest =
                image_delta.options.magnification == epaint::textures::TextureFilter::Nearest;
            let wrap_mode = wrap_mode_flag(image_delta.options.wrap_mode);
            let sampler = self
                .samplers
                .entry(image_delta.options)
                .or_insert_with(|| create_sampler(image_delta.options, device));
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label,
                layout: &self.texture_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &texture.create_view(&wgpu::TextureViewDescriptor::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.texture_flag_buffers[texture_flags(nearest, wrap_mode)]
                            .as_entire_binding(),
                    },
                ],
            })
        });

        queue_write_data_to_texture(&texture, origin);

        // A full update must (re)create the texture at exactly the delta's size,
        // or glyph UVs (normalized by the CPU atlas size) will sample the wrong rows.
        debug_assert!(
            image_delta.pos.is_some() || [texture.width(), texture.height()] == [width, height],
            "egui texture {id:?}: GPU texture is {}x{} but full delta is {width}x{height}",
            texture.width(),
            texture.height(),
        );

        self.textures.insert(
            id,
            Texture {
                texture: Some(texture),
                bind_group,
                options: Some(image_delta.options),
            },
        );
    }

    pub fn free_texture(&mut self, id: &epaint::TextureId) {
        if let Some(texture) = self.textures.remove(id).and_then(|t| t.texture) {
            texture.destroy();
        }
        // A freed texture id may come back with different content; let the
        // next pairing rejection for it warn again.
        self.upload_rejections_warned.remove(id);
    }

    /// Get the WGPU texture and bind group associated to a texture that has been allocated by egui.
    ///
    /// This could be used by custom paint hooks to render images that have been added through
    /// [`epaint::Context::load_texture`](https://docs.rs/egui/latest/egui/struct.Context.html#method.load_texture).
    pub fn texture(&self, id: &epaint::TextureId) -> Option<&Texture> {
        self.textures.get(id)
    }

    /// Registers a [`wgpu::Texture`] with a [`epaint::TextureId`].
    ///
    /// This enables the application to reference the texture inside an image ui element.
    /// This effectively enables off-screen rendering inside the egui UI. Texture must have
    /// the texture format [`wgpu::TextureFormat::Rgba8Unorm`].
    pub fn register_native_texture(
        &mut self,
        device: &wgpu::Device,
        texture: &wgpu::TextureView,
        texture_filter: wgpu::FilterMode,
    ) -> epaint::TextureId {
        self.register_native_texture_with_sampler_options(
            device,
            texture,
            wgpu::SamplerDescriptor {
                label: Some(format!("egui_user_image_{}", self.next_user_texture_id).as_str()),
                mag_filter: texture_filter,
                min_filter: texture_filter,
                ..Default::default()
            },
        )
    }

    /// Registers a [`wgpu::Texture`] with an existing [`epaint::TextureId`].
    ///
    /// This enables applications to reuse [`epaint::TextureId`]s.
    pub fn update_egui_texture_from_wgpu_texture(
        &mut self,
        device: &wgpu::Device,
        texture: &wgpu::TextureView,
        texture_filter: wgpu::FilterMode,
        id: epaint::TextureId,
    ) {
        self.update_egui_texture_from_wgpu_texture_with_sampler_options(
            device,
            texture,
            wgpu::SamplerDescriptor {
                label: Some(format!("egui_user_image_{}", self.next_user_texture_id).as_str()),
                mag_filter: texture_filter,
                min_filter: texture_filter,
                ..Default::default()
            },
            id,
        );
    }

    /// Registers a [`wgpu::Texture`] with a [`epaint::TextureId`] while also accepting custom
    /// [`wgpu::SamplerDescriptor`] options.
    ///
    /// This allows applications to specify individual minification/magnification filters as well as
    /// custom mipmap and tiling options.
    ///
    /// The texture must have the format [`wgpu::TextureFormat::Rgba8Unorm`].
    /// Any compare function supplied in the [`wgpu::SamplerDescriptor`] will be ignored.
    #[expect(clippy::needless_pass_by_value)] // false positive
    pub fn register_native_texture_with_sampler_options(
        &mut self,
        device: &wgpu::Device,
        texture: &wgpu::TextureView,
        sampler_descriptor: wgpu::SamplerDescriptor<'_>,
    ) -> epaint::TextureId {
        profiling::function_scope!();

        let nearest = sampler_descriptor.mag_filter == wgpu::FilterMode::Nearest;
        let wrap_mode = address_mode_wrap_flag(sampler_descriptor.address_mode_u);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            compare: None,
            ..sampler_descriptor
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(format!("egui_user_image_{}", self.next_user_texture_id).as_str()),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(texture),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.texture_flag_buffers[texture_flags(nearest, wrap_mode)]
                        .as_entire_binding(),
                },
            ],
        });

        let id = epaint::TextureId::User(self.next_user_texture_id);
        self.textures.insert(
            id,
            Texture {
                texture: None,
                bind_group,
                options: None,
            },
        );
        self.next_user_texture_id += 1;

        id
    }

    /// Registers a [`wgpu::Texture`] with an existing [`epaint::TextureId`] while also accepting custom
    /// [`wgpu::SamplerDescriptor`] options.
    ///
    /// This allows applications to reuse [`epaint::TextureId`]s created with custom sampler options.
    #[expect(clippy::needless_pass_by_value)] // false positive
    pub fn update_egui_texture_from_wgpu_texture_with_sampler_options(
        &mut self,
        device: &wgpu::Device,
        texture: &wgpu::TextureView,
        sampler_descriptor: wgpu::SamplerDescriptor<'_>,
        id: epaint::TextureId,
    ) {
        profiling::function_scope!();

        let Texture {
            bind_group: user_texture_binding,
            ..
        } = self
            .textures
            .get_mut(&id)
            .expect("Tried to update a texture that has not been allocated yet.");

        let nearest = sampler_descriptor.mag_filter == wgpu::FilterMode::Nearest;
        let wrap_mode = address_mode_wrap_flag(sampler_descriptor.address_mode_u);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            compare: None,
            ..sampler_descriptor
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(format!("egui_user_image_{}", self.next_user_texture_id).as_str()),
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(texture),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.texture_flag_buffers[texture_flags(nearest, wrap_mode)]
                        .as_entire_binding(),
                },
            ],
        });

        *user_texture_binding = bind_group;
    }

    /// Uploads the uniform, vertex and index data used by the renderer.
    /// Should be called before [`Self::render`].
    ///
    /// `target_format` is the format of the color attachment the upcoming render
    /// pass draws into; the pipeline for it is created here if this is its
    /// first use **under the currently active surface color space** (the cache
    /// key is (format, derived encoding) — one format can host both
    /// encodings across viewports).
    ///
    /// Returns all user-defined command buffers gathered from [`CallbackTrait::prepare`] & [`CallbackTrait::finish_prepare`] callbacks.
    pub fn update_buffers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        paint_jobs: &[epaint::ClippedPrimitive],
        screen_descriptor: &ScreenDescriptor,
        target_format: wgpu::TextureFormat,
    ) -> Vec<wgpu::CommandBuffer> {
        profiling::function_scope!();

        self.ensure_pipeline(device, target_format);

        let screen_size_in_points = screen_descriptor.screen_size_in_points();

        let uniform_buffer_content = UniformBuffer {
            screen_size_in_points,
            dithering: u32::from(self.options.dithering),
            predictable_texture_filtering: u32::from(self.options.predictable_texture_filtering),
            // The linear entry point's SDR-white scale (plan §7). Only the
            // float (scRGB) target class can reach the linear entry point,
            // so every other target — including the hardware-encoding
            // `*Srgb` row — is pinned to 1.0: a stale or misreported nits
            // value can never over-brighten a non-float surface. No
            // allocation: a plain buffer write, skipped entirely while the
            // content is unchanged.
            _padding: [0; 3],
            sdr_white_scale: if is_float_target(target_format) {
                sdr_white_scale(self.sdr_white_nits)
            } else {
                1.0
            },
        };
        if uniform_buffer_content != self.previous_uniform_buffer_content {
            profiling::scope!("update uniforms");
            queue.write_buffer(
                &self.uniform_buffer,
                0,
                bytemuck::cast_slice(&[uniform_buffer_content]),
            );
            self.previous_uniform_buffer_content = uniform_buffer_content;
        }

        // Determine how many vertices & indices need to be rendered, and gather prepare callbacks
        let mut callbacks = Vec::new();
        let (vertex_count, index_count) = {
            profiling::scope!("count_vertices_indices");
            paint_jobs.iter().fold((0, 0), |acc, clipped_primitive| {
                match &clipped_primitive.primitive {
                    Primitive::Mesh(mesh) => {
                        (acc.0 + mesh.vertices.len(), acc.1 + mesh.indices.len())
                    }
                    Primitive::Callback(callback) => {
                        if let Some(c) = callback.callback.downcast_ref::<Callback>() {
                            callbacks.push(c.0.as_ref());
                        } else {
                            log::warn!("Unknown paint callback: expected `egui_wgpu::Callback`");
                        }
                        acc
                    }
                }
            })
        };

        if index_count > 0 {
            profiling::scope!("indices", index_count.to_string().as_str());

            self.index_buffer.slices.clear();

            let required_index_buffer_size = (core::mem::size_of::<u32>() * index_count) as u64;
            if self.index_buffer.capacity < required_index_buffer_size {
                // Resize index buffer if needed.
                self.index_buffer.capacity =
                    (self.index_buffer.capacity * 2).at_least(required_index_buffer_size);
                self.index_buffer.buffer = create_index_buffer(device, self.index_buffer.capacity);
            }

            let index_buffer_staging = queue.write_buffer_with(
                &self.index_buffer.buffer,
                0,
                #[expect(clippy::unwrap_used)] // Checked above
                NonZeroU64::new(required_index_buffer_size).unwrap(),
            );

            let Some(mut index_buffer_staging) = index_buffer_staging else {
                panic!(
                    "Failed to create staging buffer for index data. Index count: {index_count}. Required index buffer size: {required_index_buffer_size}. Actual size {} and capacity: {} (bytes)",
                    self.index_buffer.buffer.size(),
                    self.index_buffer.capacity
                );
            };

            let mut index_offset = 0;
            for epaint::ClippedPrimitive { primitive, .. } in paint_jobs {
                match primitive {
                    Primitive::Mesh(mesh) => {
                        let size = mesh.indices.len() * core::mem::size_of::<u32>();
                        let slice = index_offset..(size + index_offset);
                        index_buffer_staging
                            .slice(slice.clone())
                            .copy_from_slice(bytemuck::cast_slice(&mesh.indices));
                        self.index_buffer.slices.push(slice);
                        index_offset += size;
                    }
                    Primitive::Callback(_) => {}
                }
            }
        }
        if vertex_count > 0 {
            profiling::scope!("vertices", vertex_count.to_string().as_str());

            self.vertex_buffer.slices.clear();

            let required_vertex_buffer_size =
                (core::mem::size_of::<Vertex>() * vertex_count) as u64;
            if self.vertex_buffer.capacity < required_vertex_buffer_size {
                // Resize vertex buffer if needed.
                self.vertex_buffer.capacity =
                    (self.vertex_buffer.capacity * 2).at_least(required_vertex_buffer_size);
                self.vertex_buffer.buffer =
                    create_vertex_buffer(device, self.vertex_buffer.capacity);
            }

            let vertex_buffer_staging = queue.write_buffer_with(
                &self.vertex_buffer.buffer,
                0,
                #[expect(clippy::unwrap_used)] // Checked above
                NonZeroU64::new(required_vertex_buffer_size).unwrap(),
            );

            let Some(mut vertex_buffer_staging) = vertex_buffer_staging else {
                panic!(
                    "Failed to create staging buffer for vertex data. Vertex count: {vertex_count}. Required vertex buffer size: {required_vertex_buffer_size}. Actual size {} and capacity: {} (bytes)",
                    self.vertex_buffer.buffer.size(),
                    self.vertex_buffer.capacity
                );
            };

            let mut vertex_offset = 0;
            for epaint::ClippedPrimitive { primitive, .. } in paint_jobs {
                match primitive {
                    Primitive::Mesh(mesh) => {
                        let size = mesh.vertices.len() * core::mem::size_of::<Vertex>();
                        let slice = vertex_offset..(size + vertex_offset);
                        vertex_buffer_staging
                            .slice(slice.clone())
                            .copy_from_slice(bytemuck::cast_slice(&mesh.vertices));
                        self.vertex_buffer.slices.push(slice);
                        vertex_offset += size;
                    }
                    Primitive::Callback(_) => {}
                }
            }
        }

        let mut user_cmd_bufs = Vec::new();
        {
            profiling::scope!("prepare callbacks");
            for callback in &callbacks {
                user_cmd_bufs.extend(callback.prepare(
                    device,
                    queue,
                    target_format,
                    screen_descriptor,
                    encoder,
                    &mut self.callback_resources,
                ));
            }
        }
        {
            profiling::scope!("finish prepare callbacks");
            for callback in &callbacks {
                user_cmd_bufs.extend(callback.finish_prepare(
                    device,
                    queue,
                    encoder,
                    &mut self.callback_resources,
                ));
            }
        }

        user_cmd_bufs
    }
}

// ----------------------------------------------------------------------------
// Content↔surface upload pairing (plan §4's pairing table, §5's design rule)
// ----------------------------------------------------------------------------

/// wgpu's `TEXEL_COPY_BUFFER_HEIGHT`/`MAP` row alignment in bytes; uploads
/// whose row pitch is a multiple of it need no internal staging pass.
const WGPU_BYTES_PER_ROW_ALIGNMENT: u32 = 256;

/// One texture upload, after the pairing gate: raw texel bytes, their pitch
/// contribution, and the GPU format to allocate for them.
struct UploadData<'a> {
    bytes: &'a [u8],
    bytes_per_pixel: u32,
    format: wgpu::TextureFormat,
}

/// Why a float-content upload was rejected by the pairing table.
///
/// Rendered into the once-per-texture-id `log::warn!`; egui's standard
/// placeholder renders instead of the image — never a panic, and
/// [`Renderer::update_texture`]'s signature is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PairingRejection {
    /// The content claims primaries the active surface cannot honor: showing
    /// the values as-is would gamut-clip them, and there is no honest guess.
    GamutClipped,
    /// The active surface pairs a linear-storage color space (scRGB), where
    /// only the sRGB tag rides natively (the linear pipelines decode it);
    /// this content's primaries would need a gamut conversion egui does not
    /// ship yet (the §5 matrices work).
    LinearStorageNotPairedYet,
    /// The surface's color space is not resolved (`Auto`), so its
    /// interpretation depends on the format: no honest row for tagged
    /// content.
    UnresolvedSurfaceSpace,
}

impl core::fmt::Display for PairingRejection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            // The evaluated (content primaries, surface color space) pair is
            // appended by the warn site, so this reason stays honest for
            // every row that maps to it (sRGB spaces, unresolved BT.2100).
            Self::GamutClipped => {
                "tagged float content has no native pairing on this surface's color space: \
                 showing the values as-is would re-interpret the tag (gamut-clip or wrong \
                 primaries), and egui does not silently re-interpret tagged content. Skipping \
                 the upload (the placeholder renders). Re-encode the image app-side, or grant \
                 the viewport a surface whose color space honors the tag \
                 (SurfacePreference::WideGamut)"
            }
            Self::LinearStorageNotPairedYet => {
                "this surface's color space (scRGB-linear) stores linear values and only \
                 sRGB-tagged content pairs natively there (egui's linear pipelines decode it); \
                 this content's primaries would need a gamut conversion egui does not ship yet. \
                 Skipping the upload (the placeholder renders). Re-encode the image app-side, or \
                 grant the viewport a surface whose color space honors the tag"
            }
            Self::UnresolvedSurfaceSpace => {
                "this surface's color space is unresolved (Auto), so tagged float content has no \
                 honest interpretation on it. Skipping the upload (the placeholder renders)"
            }
        };
        f.write_str(text)
    }
}

/// The content↔surface pairing table for float content: does a
/// [`epaint::FloatImage`] with `content_primaries` upload natively onto a
/// surface whose granted color space is `active_surface_color_space`?
///
/// This function IS the pairing table (plan §4) — the one place outside
/// `Primaries` itself that names concrete color spaces for content pairing
/// (plan §5 design rule). Semantics per row:
///
/// | Content tag | Granted surface space        | Behavior            |
/// |-------------|------------------------------|---------------------|
/// | `Srgb`      | sRGB space (any format)      | native              |
/// | `Srgb`      | `DisplayP3`                  | native (shifts like the chrome) |
/// | `Srgb`      | `ExtendedSrgbLinear` (scRGB) | native (the linear pipelines decode; §7 "app-converted content rides the sRGB basis tag") |
/// | `DisplayP3` | `DisplayP3` (± extended)     | native              |
/// | `DisplayP3` | sRGB / extended-sRGB         | loud reject         |
/// | `DisplayP3` | scRGB-linear                 | loud reject (the primaries conversion ships with §5's matrices) |
///
/// `None` (no surface known — app-driven/headless uploads) pairs with
/// today's SDR-sRGB semantics.
///
/// The arms mirror the plan's table rows 1:1 on purpose; some bodies
/// coincide, and merging them would erase the row structure.
#[expect(clippy::match_same_arms)]
fn pair_float_content_with_surface(
    content_primaries: epaint::Primaries,
    active_surface_color_space: Option<wgpu::SurfaceColorSpace>,
) -> Result<(), PairingRejection> {
    use wgpu::SurfaceColorSpace as Scs;

    let space = active_surface_color_space.unwrap_or(Scs::Srgb);
    match (content_primaries, space) {
        // sRGB content follows the surface's interpretation, the same shift
        // the chrome gets on a P3-referred surface (§4 row 2; §5). On the
        // scRGB pairing the sRGB tag rides natively: the linear entry point
        // decodes the gamma-encoded values, so no conversion is needed here
        // (§7).
        (
            epaint::Primaries::Srgb,
            Scs::Auto
            | Scs::Srgb
            | Scs::ExtendedSrgb
            | Scs::ExtendedSrgbLinear
            | Scs::DisplayP3
            | Scs::ExtendedDisplayP3,
        ) => Ok(()),

        // Explicit P3 is native only on P3 spaces (± extended).
        (epaint::Primaries::DisplayP3, Scs::DisplayP3 | Scs::ExtendedDisplayP3) => Ok(()),

        // An explicit claim the surface cannot honor is gamut-clipped there:
        // no silent guess is honest (§4).
        (epaint::Primaries::DisplayP3, Scs::Srgb | Scs::ExtendedSrgb) => {
            Err(PairingRejection::GamutClipped)
        }

        // `Auto` is unresolved: its interpretation depends on the format
        // (sRGB on 8-bit, scRGB-linear on fp16), so tagged content has no
        // honest row.
        (epaint::Primaries::DisplayP3, Scs::Auto) => Err(PairingRejection::UnresolvedSurfaceSpace),

        // P3 content on scRGB-linear needs the primaries conversion in
        // linear light (§7), which ships with §5's matrices work — until
        // then the sRGB tag is the only one riding this pairing.
        (epaint::Primaries::DisplayP3, Scs::ExtendedSrgbLinear) => {
            Err(PairingRejection::LinearStorageNotPairedYet)
        }

        // Totality: BT.2100 surfaces are not grantable in v1; tagged content
        // has no honest row on them.
        (_, Scs::Bt2100Pq | Scs::Bt2100Hlg) => Err(PairingRejection::GamutClipped),
    }
}

/// Set in bit 0 of the texture flags if the sampler uses nearest filtering.
///
/// Must match `TEX_FLAG_NEAREST` in `egui.wgsl`.
const TEX_FLAG_NEAREST: u32 = 1;

/// Wrap modes, stored in bits 1+ of the texture flags.
///
/// Must match the `WRAP_MODE_*` constants in `egui.wgsl`.
const WRAP_MODE_CLAMP_TO_EDGE: u32 = 0;
const WRAP_MODE_REPEAT: u32 = 1;
const WRAP_MODE_MIRRORED_REPEAT: u32 = 2;

/// Number of distinct values [`texture_flags`] can return.
const NUM_TEXTURE_FLAGS: usize = 6;

fn wrap_mode_flag(wrap_mode: epaint::textures::TextureWrapMode) -> u32 {
    match wrap_mode {
        epaint::textures::TextureWrapMode::ClampToEdge => WRAP_MODE_CLAMP_TO_EDGE,
        epaint::textures::TextureWrapMode::Repeat => WRAP_MODE_REPEAT,
        epaint::textures::TextureWrapMode::MirroredRepeat => WRAP_MODE_MIRRORED_REPEAT,
    }
}

fn address_mode_wrap_flag(address_mode: wgpu::AddressMode) -> u32 {
    match address_mode {
        wgpu::AddressMode::Repeat => WRAP_MODE_REPEAT,
        wgpu::AddressMode::MirrorRepeat => WRAP_MODE_MIRRORED_REPEAT,
        wgpu::AddressMode::ClampToEdge | wgpu::AddressMode::ClampToBorder => {
            WRAP_MODE_CLAMP_TO_EDGE
        }
    }
}

/// Index into [`Renderer::texture_flag_buffers`]: the texture flags
/// read by the shader when `predictable_texture_filtering` is on.
///
/// Bit 0: [`TEX_FLAG_NEAREST`].
/// Bits 1+: one of the `WRAP_MODE_*` constants.
fn texture_flags(nearest: bool, wrap_mode: u32) -> usize {
    let nearest = if nearest { TEX_FLAG_NEAREST } else { 0 };
    (nearest | (wrap_mode << 1)) as usize
}

/// The float render-target formats whose pipelines never dither.
///
/// A float target has no 8-bit quantizer for the dither noise to hide in; on
/// such targets the noise is harmful, not meaningless (plan §3), so their
/// pipelines are built from the dither-free shader module regardless of
/// [`RendererOptions::dithering`]. 8-bit targets keep today's behavior
/// bit-for-bit.
///
/// Table data: extend when egui renders to more float target formats (keep in
/// sync with `is_f16` in `surface_policy.rs`, the HDR pairing class —
/// enforced by `surface_policy`'s `float_target_tables_agree` test).
pub(crate) fn is_float_target(format: wgpu::TextureFormat) -> bool {
    matches!(format, wgpu::TextureFormat::Rgba16Float)
}

/// How the fragment path must write color for a surface to encode exactly
/// once (plan §3).
///
/// A derived quantity: computed from `(format, color_space)` wherever an
/// entry point or cache key is selected — never stored next to its inputs
/// (storing a derivable value re-introduces drift when either input changes
/// under it, e.g. on a monitor re-decide).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OutputEncoding {
    /// The fragment writes gamma-encoded values (the passthrough entry
    /// point); the surface stores them as-is. The u8 raw-view grammar and
    /// the fp16 grammar on every pairing except the scRGB one.
    GammaEncoded,

    /// The fragment writes linear values (the decode entry point); encoding
    /// happens downstream, in the format's hardware or the presentation
    /// engine (scRGB composites in linear).
    Linear,
}

impl OutputEncoding {
    /// The encoding a render pass into `(format, color_space)` must use.
    ///
    /// Total per plan §3's table — every legal pairing maps to an encoding,
    /// and the totality is unit-tested. The rows:
    ///
    /// | Format class                  | Color space              | Encoding       |
    /// |-------------------------------|--------------------------|----------------|
    /// | u8 raw views (`Bgra8Unorm` …) | any                      | `GammaEncoded` |
    /// | fp16 (`Rgba16Float`)          | non-extended (incl. `Srgb`, `Auto`) | `GammaEncoded` |
    /// | fp16                          | `ExtendedSrgb`           | `GammaEncoded` |
    /// | fp16                          | `DisplayP3` / `ExtendedDisplayP3` | `GammaEncoded` |
    /// | fp16                          | `ExtendedSrgbLinear`     | `Linear`       |
    ///
    /// The scRGB row is what makes the pairing grantable: the surface stores
    /// linear values (Windows' DWM composites in linear scRGB, plan §7), so
    /// the fragment must decode the gamma-encoded contract before writing.
    /// Ungrantable combos (u8 with `ExtendedSrgbLinear`, wider floats) read
    /// as `GammaEncoded` — the table's rows, not a guess.
    ///
    /// `*Srgb` formats are not an egui pairing (the decider grants raw views
    /// only), but the function is total over formats: a hardware-encoding
    /// format demands linear input (it encodes exactly once, on store) — the
    /// same row today's `TextureFormat::is_srgb()` sniff selected, kept so
    /// non-egui surfaces keep their entry point.
    #[must_use]
    pub fn of(format: wgpu::TextureFormat, color_space: wgpu::SurfaceColorSpace) -> Self {
        use wgpu::SurfaceColorSpace as Scs;

        if format.is_srgb() {
            // Hardware encodes on store; the fragment writes linear (the
            // row today's `is_srgb()` selection picks — kept).
            return Self::Linear;
        }
        match (is_float_target(format), color_space) {
            // The scRGB pairing (fp16 only, the grantable class): the
            // surface stores linear values — decode in shader.
            (true, Scs::ExtendedSrgbLinear) => Self::Linear,
            // Every other raw-view pairing — u8 wide-gamut SDR, fp16
            // extended sRGB / extended Display P3, fp16 with a non-extended
            // space (`Srgb` included, legal in wgpu), unresolved BT.2100 —
            // stores the gamma-encoded contract as-is.
            _ => Self::GammaEncoded,
        }
    }
}

fn create_sampler(
    options: epaint::textures::TextureOptions,
    device: &wgpu::Device,
) -> wgpu::Sampler {
    let mag_filter = match options.magnification {
        epaint::textures::TextureFilter::Nearest => wgpu::FilterMode::Nearest,
        epaint::textures::TextureFilter::Linear => wgpu::FilterMode::Linear,
    };
    let min_filter = match options.minification {
        epaint::textures::TextureFilter::Nearest => wgpu::FilterMode::Nearest,
        epaint::textures::TextureFilter::Linear => wgpu::FilterMode::Linear,
    };
    let address_mode = match options.wrap_mode {
        epaint::textures::TextureWrapMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        epaint::textures::TextureWrapMode::Repeat => wgpu::AddressMode::Repeat,
        epaint::textures::TextureWrapMode::MirroredRepeat => wgpu::AddressMode::MirrorRepeat,
    };
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(&format!(
            "egui sampler (mag: {mag_filter:?}, min {min_filter:?})"
        )),
        mag_filter,
        min_filter,
        address_mode_u: address_mode,
        address_mode_v: address_mode,
        ..Default::default()
    })
}

/// Creates the egui render pipeline for one render-pass target format.
///
/// The fragment entry point comes from the derived [`OutputEncoding`] (plan
/// §3): `GammaEncoded` passes the gamma-encoded texel×vertex product
/// through, `Linear` decodes it for surfaces that store linear values
/// (scRGB) or encode in hardware (`*Srgb` formats). The two dither module
/// variants × the two entry points stay one source of truth: both live in
/// the same concatenated module, selected here, never copied.
fn create_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    options: &RendererOptions,
    target_format: wgpu::TextureFormat,
    encoding: OutputEncoding,
) -> wgpu::RenderPipeline {
    profiling::scope!("create_render_pipeline");

    let depth_stencil = options
        .depth_stencil_format
        .map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("egui_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            entry_point: Some("vs_main"),
            module,
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 5 * 4,
                step_mode: wgpu::VertexStepMode::Vertex,
                // 0: vec2 position
                // 1: vec2 texture coordinates
                // 2: uint color
                attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Uint32],
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default()
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            unclipped_depth: false,
            conservative: false,
            cull_mode: None,
            front_face: wgpu::FrontFace::default(),
            polygon_mode: wgpu::PolygonMode::default(),
            strip_index_format: None,
        },
        depth_stencil,
        multisample: wgpu::MultisampleState {
            alpha_to_coverage_enabled: false,
            count: options.msaa_samples.max(1),
            mask: !0,
        },

        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(match encoding {
                // The kept advice for hardware-encoding formats (not an egui
                // pairing — the decider grants raw views only): egui prefers
                // the raw 8-bit formats.
                OutputEncoding::Linear => {
                    if target_format.is_srgb() {
                        log::warn!("Detected a linear (sRGBA aware) framebuffer {target_format:?}. egui prefers Rgba8Unorm or Bgra8Unorm");
                    }
                    "fs_main_linear_framebuffer"
                }
                OutputEncoding::GammaEncoded => {
                    "fs_main_gamma_framebuffer" // this is what we prefer
                }
            }),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default()
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_vertex_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    profiling::function_scope!();
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("egui_vertex_buffer"),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        size,
        mapped_at_creation: false,
    })
}

fn create_index_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    profiling::function_scope!();
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("egui_index_buffer"),
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        size,
        mapped_at_creation: false,
    })
}

/// A Rect in physical pixel space, used for setting clipping rectangles.
struct ScissorRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl ScissorRect {
    fn new(clip_rect: &epaint::Rect, pixels_per_point: f32, target_size: [u32; 2]) -> Self {
        // Transform clip rect to physical pixels:
        let clip_min_x = pixels_per_point * clip_rect.min.x;
        let clip_min_y = pixels_per_point * clip_rect.min.y;
        let clip_max_x = pixels_per_point * clip_rect.max.x;
        let clip_max_y = pixels_per_point * clip_rect.max.y;

        // Round to integer:
        let clip_min_x = clip_min_x.round() as u32;
        let clip_min_y = clip_min_y.round() as u32;
        let clip_max_x = clip_max_x.round() as u32;
        let clip_max_y = clip_max_y.round() as u32;

        // Clamp:
        let clip_min_x = clip_min_x.clamp(0, target_size[0]);
        let clip_min_y = clip_min_y.clamp(0, target_size[1]);
        let clip_max_x = clip_max_x.clamp(clip_min_x, target_size[0]);
        let clip_max_y = clip_max_y.clamp(clip_min_y, target_size[1]);

        Self {
            x: clip_min_x,
            y: clip_min_y,
            width: clip_max_x - clip_min_x,
            height: clip_max_y - clip_min_y,
        }
    }
}

// Compile-time check that `Renderer` is `Send + Sync`.
// Deliberately not a `#[test]`: tests never run on wasm, but `cargo check` does.
// On wasm this only holds with `fragile-send-sync-non-atomic-wasm` and without threads;
// look at the feature flag for an explanation.
#[cfg(any(
    not(target_arch = "wasm32"),
    all(
        feature = "fragile-send-sync-non-atomic-wasm",
        not(target_feature = "atomics")
    ),
))]
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Renderer>();
};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    use core::time::Duration;

    /// Renders one empty egui frame offscreen through [`Renderer::update_buffers`]
    /// and [`Renderer::render`], as a real render pass onto a real texture.
    fn render_offscreen(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut Renderer,
        target_format: wgpu::TextureFormat,
    ) {
        const SIZE: u32 = 64;

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("egui-wgpu pipeline cache test"),
        });
        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: [SIZE, SIZE],
            pixels_per_point: 1.0,
        };
        renderer.update_buffers(
            device,
            queue,
            &mut encoder,
            &[],
            &screen_descriptor,
            target_format,
        );

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("egui-wgpu pipeline cache test target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: target_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        {
            let mut render_pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui-wgpu pipeline cache test pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            renderer.render(&mut render_pass, &[], &screen_descriptor, target_format);
        }

        queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(10)),
            })
            .expect("GPU poll failed while waiting for the pipeline cache test frame");
    }

    /// The pipeline cache is keyed by (target format, derived encoding):
    /// repeated renders under one key reuse one pipeline, a second format (or
    /// encoding) gets its own entry, and no device error surfaces along the
    /// way (plan §8, row 1; plan §3's encoding-keyed cache).
    ///
    /// Adapter-requiring: this is egui-wgpu's first runtime test. Machines
    /// without any wgpu adapter skip with a message; CI and normal dev
    /// machines (hardware GPU or software rasterizer) always run it.
    #[test]
    fn pipeline_cache_is_keyed_by_target_format_and_encoding() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = match pollster::block_on(instance.request_adapter(
            &wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            },
        )) {
            Ok(adapter) => adapter,
            Err(err) => {
                // Machines without any wgpu adapter skip; CI (macos) and dev
                // machines with GPUs (or software rasterizers) always run this.
                #[expect(clippy::print_stderr)]
                {
                    eprintln!(
                        "skipping pipeline_cache_is_keyed_by_target_format_and_encoding: no wgpu adapter: {err}"
                    );
                }
                return;
            }
        };

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("egui-wgpu pipeline cache test"),
            ..Default::default()
        }))
        .expect("failed to request a wgpu device for the pipeline cache test");

        // Any device error (validation or otherwise) fails the test.
        device.on_uncaptured_error(Arc::new(|err| {
            panic!("wgpu device error during the pipeline cache test: {err}");
        }));

        let mut renderer = Renderer::new(&device, RendererOptions::default());
        assert!(
            renderer.pipelines.lock().is_empty(),
            "pipelines must be created lazily, not in Renderer::new"
        );
        // No active surface: Srgb semantics → the GammaEncoded entry point.
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::Srgb));

        render_offscreen(
            &device,
            &queue,
            &mut renderer,
            wgpu::TextureFormat::Bgra8Unorm,
        );
        let first_bgra = renderer
            .pipelines
            .lock()
            .get(&(
                wgpu::TextureFormat::Bgra8Unorm,
                OutputEncoding::GammaEncoded,
            ))
            .cloned()
            .expect("the Bgra8Unorm pipeline must be cached after the first render");

        render_offscreen(
            &device,
            &queue,
            &mut renderer,
            wgpu::TextureFormat::Bgra8Unorm,
        );

        {
            let pipelines = renderer.pipelines.lock();
            assert_eq!(
                pipelines.len(),
                1,
                "repeated renders at the same format must not add cache entries"
            );
            assert!(
                Arc::ptr_eq(
                    &first_bgra,
                    pipelines
                        .get(&(
                            wgpu::TextureFormat::Bgra8Unorm,
                            OutputEncoding::GammaEncoded
                        ))
                        .expect("the Bgra8Unorm cache entry must survive")
                ),
                "repeated renders at the same format must reuse the cached pipeline"
            );
        }

        render_offscreen(
            &device,
            &queue,
            &mut renderer,
            wgpu::TextureFormat::Rgba8Unorm,
        );

        let pipelines = renderer.pipelines.lock();
        assert_eq!(
            pipelines.len(),
            2,
            "a second target format must get its own cache entry"
        );
        assert!(
            !Arc::ptr_eq(
                &first_bgra,
                pipelines
                    .get(&(
                        wgpu::TextureFormat::Rgba8Unorm,
                        OutputEncoding::GammaEncoded
                    ))
                    .expect("the Rgba8Unorm pipeline must be cached"),
            ),
            "distinct target formats must hold distinct pipelines"
        );
    }

    /// One format, both encodings (plan §3): an fp16 target reaching
    /// `ExtendedSrgbLinear` (the scRGB pairing) alongside every other space
    /// legitimately hosts two pipelines — the encoding is part of the cache
    /// key, so neither evicts the other and repeats reuse their own entry.
    /// Adapter-requiring (2a conventions).
    #[test]
    fn pipeline_cache_keys_both_encodings_per_format() {
        let Some((device, queue)) = test_device("pipeline_cache_keys_both_encodings_per_format")
        else {
            return;
        };

        let mut renderer = Renderer::new(&device, RendererOptions::default());

        // First encoding: the sRGB row (every non-scRGB pairing) on fp16.
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::ExtendedSrgb));
        render_offscreen(
            &device,
            &queue,
            &mut renderer,
            wgpu::TextureFormat::Rgba16Float,
        );

        // Second encoding: the scRGB pairing on the same target format.
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::ExtendedSrgbLinear));
        render_offscreen(
            &device,
            &queue,
            &mut renderer,
            wgpu::TextureFormat::Rgba16Float,
        );

        let (gamma, linear) = {
            let pipelines = renderer.pipelines.lock();
            assert_eq!(
                pipelines.len(),
                2,
                "one format may host one pipeline per encoding, no more"
            );
            let gamma = Arc::clone(
                pipelines
                    .get(&(
                        wgpu::TextureFormat::Rgba16Float,
                        OutputEncoding::GammaEncoded,
                    ))
                    .expect("the GammaEncoded fp16 pipeline must be cached"),
            );
            let linear = Arc::clone(
                pipelines
                    .get(&(wgpu::TextureFormat::Rgba16Float, OutputEncoding::Linear))
                    .expect("the Linear fp16 pipeline must be cached"),
            );
            (gamma, linear)
        };
        assert!(
            !Arc::ptr_eq(&gamma, &linear),
            "the two encodings must be distinct pipelines"
        );

        // Repeats under each encoding reuse their own entry (no third
        // pipeline, no recompilation churn).
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::ExtendedSrgb));
        render_offscreen(
            &device,
            &queue,
            &mut renderer,
            wgpu::TextureFormat::Rgba16Float,
        );
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::ExtendedSrgbLinear));
        render_offscreen(
            &device,
            &queue,
            &mut renderer,
            wgpu::TextureFormat::Rgba16Float,
        );
        let pipelines = renderer.pipelines.lock();
        assert_eq!(pipelines.len(), 2);
        assert!(Arc::ptr_eq(
            &gamma,
            pipelines
                .get(&(
                    wgpu::TextureFormat::Rgba16Float,
                    OutputEncoding::GammaEncoded
                ))
                .expect("the GammaEncoded entry must survive")
        ));
        assert!(Arc::ptr_eq(
            &linear,
            pipelines
                .get(&(wgpu::TextureFormat::Rgba16Float, OutputEncoding::Linear))
                .expect("the Linear entry must survive")
        ));
    }

    /// Guards the module split against drift at the name level.
    ///
    /// If a copy of the transfer functions (or a renamed re-implementation)
    /// crept back into `egui.wgsl`, nothing would fail at build time — WGSL
    /// is only parsed when a real device compiles the concatenated module in
    /// `Renderer::new`, so duplicate names would only surface there, on a
    /// GPU machine, and renamed local copies would never fail at all, silently
    /// drifting from the shared file. This check keeps `egui.wgsl` free of
    /// transfer-function *definitions* so the functions the round-trip test
    /// exercises stay the functions production renders with — and keeps the
    /// no-`pow`-on-negatives property auditable in the one shared file.
    #[test]
    fn egui_wgsl_does_not_inline_transfer_functions() {
        let wgsl = include_str!("egui.wgsl");
        for name in [
            "fn linear_from_gamma_rgb",
            "fn gamma_from_linear_rgb",
            "fn gamma_from_linear_rgba",
        ] {
            assert!(
                !wgsl.contains(name),
                "`egui.wgsl` must not define `{name}`: the transfer functions live \
                 in `egui_transfer.wgsl`, which the renderer concatenates and the \
                 round-trip test exercises"
            );
        }
    }

    /// Test entry points concatenated with `egui_transfer.wgsl` — the exact
    /// shared-helpers file `Renderer::new` concatenates into the production
    /// module, so the tested functions cannot drift from the shipped ones.
    ///
    /// One instance per input covers one pixel of an 8×8 grid; a single draw
    /// fills three `Rgba16Float` targets with the decode, the encode, and
    /// their composition.
    const TRANSFER_TEST_WGSL: &str = r"
struct TestInputs {
    // Must equal `NUM_TRANSFER_TEST_INPUTS` in `transfer_test_inputs`.
    values: array<vec4<f32>, 32>,
}
@group(0) @binding(0) var<uniform> r_test_inputs: TestInputs;

// Must equal the test target size.
const GRID: u32 = 8u;

struct TestVOut {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) index: u32,
}

@vertex
fn vs_test(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> TestVOut {
    let pixel = vec2<f32>(f32(ii % GRID), f32(ii / GRID));
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let xy = (pixel + corner) * 2.0 / f32(GRID);
    return TestVOut(vec4<f32>(xy.x - 1.0, 1.0 - xy.y, 0.0, 1.0), ii);
}

struct TestFOut {
    @location(0) decoded: vec4<f32>,
    @location(1) encoded: vec4<f32>,
    @location(2) round_trip: vec4<f32>,
}

@fragment
fn fs_test(in: TestVOut) -> TestFOut {
    let v = r_test_inputs.values[in.index].rgb;
    let encoded = gamma_from_linear_rgb(v);
    return TestFOut(
        vec4<f32>(linear_from_gamma_rgb(v), 1.0),
        vec4<f32>(encoded, 1.0),
        vec4<f32>(linear_from_gamma_rgb(encoded), 1.0),
    );
}
";

    /// The round-trip input set (plan §8 row 1): positives across the domain,
    /// the exact toe boundaries (`0.0031308`, `0.04045`), both zeros,
    /// negatives (deep toe, knee, power region), and mixed-sign vec3 lanes
    /// for per-lane select correctness. Every input feeds all three targets.
    const NUM_TRANSFER_TEST_INPUTS: usize = 32;

    fn transfer_test_inputs() -> Vec<[f32; 4]> {
        let scalars: [f32; 27] = [
            // Positives across the domain.
            0.0001,
            0.001,
            0.003_130_7, // just below the encode knee
            0.003_130_8, // the encode knee itself
            0.003_130_9, // just above it
            0.01,
            0.040_44, // just below the decode knee
            0.040_45, // the decode knee itself
            0.040_46, // just above it
            0.1,
            0.25,
            0.5,
            0.75,
            1.0, // SDR reference white
            2.0,
            4.0,
            10.0,
            // Both zeros — `-0.0` must produce `-0.0`-symmetric results, never NaN.
            0.0,
            -0.0,
            // Negatives: deep toe, toe, knee, mid, power region.
            -0.0001,
            -0.001,
            -0.003_130_8,
            -0.01,
            -0.040_45,
            -0.5,
            -1.0,
            -4.0,
        ];
        let mixed_sign: [[f32; 3]; 5] = [
            [0.5, -0.5, 0.02],
            [-1.0, 0.001, -0.003_130_8],
            [4.0, -4.0, 1.0],
            [-0.0, 0.0, 0.5],
            [0.003_130_8, -0.040_45, 2.0],
        ];
        let mut inputs = Vec::with_capacity(NUM_TRANSFER_TEST_INPUTS);
        for &v in &scalars {
            inputs.push([v, v, v, 0.0]);
        }
        for m in mixed_sign {
            inputs.push([m[0], m[1], m[2], 0.0]);
        }
        assert_eq!(
            inputs.len(),
            NUM_TRANSFER_TEST_INPUTS,
            "the input list must fill the WGSL uniform array exactly"
        );
        inputs
    }

    /// Smallest positive normal `f16` (= 2⁻¹⁴); below it half-precision steps
    /// are subnormal-sized, so a relative tolerance would explode.
    const F16_MIN_NORMAL: f32 = 6.103_515_6e-5;

    /// Plan §8 tolerance: one full f16 quantization step of the CPU value in
    /// the normal range (2⁻¹⁰ relative; round-to-nearest would only ever be
    /// half a step, but the f16 stores of at least Apple-silicon Metal round
    /// toward zero, so the bound must absorb a full step), plus an absolute
    /// carve-out in the f16 subnormal range where a relative bound is
    /// meaningless.
    fn assert_f16_close(gpu: f32, cpu: f32, context: &str) {
        assert!(
            !gpu.is_nan(),
            "GPU result must never be NaN ({context}): GPU {gpu}, CPU {cpu}"
        );
        let diff = (gpu - cpu).abs();
        let limit = if cpu.abs() < F16_MIN_NORMAL {
            1.192_092_9e-7 // 2⁻²³: a couple of f16 subnormal steps (2⁻²⁴ each)
        } else {
            cpu.abs() * 9.765_625e-4 // 2⁻¹⁰: one full f16 step
        };
        assert!(
            diff <= limit,
            "GPU/CPU mismatch ({context}): GPU {gpu}, CPU {cpu}, |Δ| {diff}, limit {limit}"
        );
    }

    fn read_f16_pixels(
        device: &wgpu::Device,
        buffer: &wgpu::Buffer,
        size: u32,
        bytes_per_row: u32,
    ) -> Vec<[f32; 4]> {
        crate::readback::texture_to_float_image(device, buffer, [size, size], bytes_per_row)
    }

    /// Round-trips the REAL production WGSL transfer functions against the
    /// CPU `ecolor` functions, negative inputs included (plan §4 "Negative
    /// extension", §8 test-matrix row 1).
    ///
    /// Asserts, per input lane on `Rgba16Float` targets (i.e. genuinely f16
    /// quantized, as a real egui frame would store them):
    /// - the GPU result is never NaN (the regression the `sign·f(|x|)` fix
    ///   guards against on drivers that lower `select` to arithmetic blends),
    /// - decode/encode match `epaint::ecolor::{linear_from_gamma,
    ///   gamma_from_linear}` within the §8 tolerance,
    /// - `decode ∘ encode ≈ identity`.
    ///
    /// Adapter-requiring like the pipeline-cache test: machines without any
    /// wgpu adapter skip with a message, never silently.
    #[test]
    fn transfer_functions_match_ecolor_on_gpu() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = match pollster::block_on(instance.request_adapter(
            &wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            },
        )) {
            Ok(adapter) => adapter,
            Err(err) => {
                // Machines without any wgpu adapter skip; CI (macos) and dev
                // machines with GPUs (or software rasterizers) always run this.
                #[expect(clippy::print_stderr)]
                {
                    eprintln!(
                        "skipping transfer_functions_match_ecolor_on_gpu: no wgpu adapter: {err}"
                    );
                }
                return;
            }
        };

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("egui-wgpu transfer round-trip test"),
            ..Default::default()
        }))
        .expect("failed to request a wgpu device for the transfer round-trip test");

        // Any device error (validation or otherwise) fails the test.
        device.on_uncaptured_error(Arc::new(|err| {
            panic!("wgpu device error during the transfer round-trip test: {err}");
        }));

        let inputs = transfer_test_inputs();

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("egui-wgpu transfer round-trip test"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!(
                "{}\n{}",
                include_str!("egui_transfer.wgsl"),
                TRANSFER_TEST_WGSL
            ))),
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("transfer round-trip test inputs"),
            contents: bytemuck::cast_slice(&inputs),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("transfer round-trip test inputs layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new((NUM_TRANSFER_TEST_INPUTS * 16) as _),
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("transfer round-trip test inputs"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        const SIZE: u32 = 8; // must equal `GRID` in `TRANSFER_TEST_WGSL`
        const BYTES_PER_ROW: u32 = 256; // 8 px × 8 B of data, padded to wgpu's alignment

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("transfer round-trip test layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("transfer round-trip test pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_test"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                unclipped_depth: false,
                conservative: false,
                cull_mode: None,
                front_face: wgpu::FrontFace::default(),
                polygon_mode: wgpu::PolygonMode::default(),
                strip_index_format: None,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                alpha_to_coverage_enabled: false,
                count: 1,
                mask: !0,
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_test"),
                compilation_options: Default::default(),
                targets: &[const {
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: None, // replace: raw function outputs, no blending
                        write_mask: wgpu::ColorWrites::ALL,
                    })
                }; 3],
            }),
            multiview_mask: None,
            cache: None,
        });

        let make_target = |label: &'static str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let decode_target = make_target("transfer round-trip test decode target");
        let encode_target = make_target("transfer round-trip test encode target");
        let round_trip_target = make_target("transfer round-trip test round-trip target");

        let make_readback = |label: &'static str| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: (BYTES_PER_ROW * SIZE) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        };
        let decode_readback = make_readback("transfer round-trip test decode readback");
        let encode_readback = make_readback("transfer round-trip test encode readback");
        let round_trip_readback = make_readback("transfer round-trip test round-trip readback");

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("transfer round-trip test encoder"),
        });
        {
            let decode_view = decode_target.create_view(&wgpu::TextureViewDescriptor::default());
            let encode_view = encode_target.create_view(&wgpu::TextureViewDescriptor::default());
            let round_trip_view =
                round_trip_target.create_view(&wgpu::TextureViewDescriptor::default());
            let mut render_pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("transfer round-trip test pass"),
                    color_attachments: &[
                        Some(wgpu::RenderPassColorAttachment {
                            view: &decode_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        }),
                        Some(wgpu::RenderPassColorAttachment {
                            view: &encode_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        }),
                        Some(wgpu::RenderPassColorAttachment {
                            view: &round_trip_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        }),
                    ],
                    ..Default::default()
                })
                .forget_lifetime();
            render_pass.set_pipeline(&pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..4, 0..NUM_TRANSFER_TEST_INPUTS as u32);
        }
        for (texture, buffer) in [
            (&decode_target, &decode_readback),
            (&encode_target, &encode_readback),
            (&round_trip_target, &round_trip_readback),
        ] {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(BYTES_PER_ROW),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
            );
        }
        queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(10)),
            })
            .expect("GPU poll failed while waiting for the transfer round-trip frame");

        let decode_pixels = read_f16_pixels(&device, &decode_readback, SIZE, BYTES_PER_ROW);
        let encode_pixels = read_f16_pixels(&device, &encode_readback, SIZE, BYTES_PER_ROW);
        let round_trip_pixels = read_f16_pixels(&device, &round_trip_readback, SIZE, BYTES_PER_ROW);

        let mut checked = 0usize;
        for (i, input) in inputs.iter().enumerate() {
            for (lane, &x) in input[..3].iter().enumerate() {
                let context = format!("input {i} lane {lane} (x = {x})");
                assert_f16_close(
                    decode_pixels[i][lane],
                    epaint::ecolor::linear_from_gamma(x),
                    &format!("decode of {context}"),
                );
                assert_f16_close(
                    encode_pixels[i][lane],
                    epaint::ecolor::gamma_from_linear(x),
                    &format!("encode of {context}"),
                );
                assert_f16_close(
                    round_trip_pixels[i][lane],
                    x,
                    &format!("decode∘encode round-trip of {context}"),
                );
                checked += 3;
            }
        }
        assert_eq!(checked, NUM_TRANSFER_TEST_INPUTS * 3 * 3);
    }

    // ------------------------------------------------------------------
    // Float content upload pairing + f16 upload (plan §8, row 2b)
    // ------------------------------------------------------------------

    /// The pairing table is total over `Primaries` × `SurfaceColorSpace` and
    /// matches plan §4's rows exactly. Pure function — runs everywhere.
    ///
    /// One arm per plan row (some bodies coincide — that's the point: the
    /// arms document the rows, not the outcomes).
    #[expect(clippy::match_same_arms)]
    #[test]
    fn pairing_table_matches_plan_rows() {
        use wgpu::SurfaceColorSpace as Scs;

        let spaces = [
            Scs::Auto,
            Scs::Srgb,
            Scs::ExtendedSrgb,
            Scs::ExtendedSrgbLinear,
            Scs::DisplayP3,
            Scs::ExtendedDisplayP3,
            Scs::Bt2100Pq,
            Scs::Bt2100Hlg,
        ];
        for primaries in [epaint::Primaries::Srgb, epaint::Primaries::DisplayP3] {
            for space in spaces {
                let result = pair_float_content_with_surface(primaries, Some(space));
                match (primaries, space) {
                    // Srgb content: native on sRGB spaces, on DisplayP3
                    // (shifts like the chrome), and on the scRGB pairing
                    // (the linear pipelines decode — PR 4's flip); rejected
                    // only on non-grantable BT.2100 spaces.
                    (
                        epaint::Primaries::Srgb,
                        Scs::Auto
                        | Scs::Srgb
                        | Scs::ExtendedSrgb
                        | Scs::ExtendedSrgbLinear
                        | Scs::DisplayP3
                        | Scs::ExtendedDisplayP3,
                    ) => {
                        assert_eq!(result, Ok(()), "{primaries:?} on {space:?}");
                    }
                    // DisplayP3 content: native on P3 (± extended), loud
                    // reject everywhere else (sRGB spaces gamut-clip; scRGB
                    // needs the §5 matrices conversion; BT.2100 not
                    // grantable).
                    (epaint::Primaries::DisplayP3, Scs::DisplayP3 | Scs::ExtendedDisplayP3) => {
                        assert_eq!(result, Ok(()), "{primaries:?} on {space:?}");
                    }
                    (epaint::Primaries::DisplayP3, Scs::Srgb | Scs::ExtendedSrgb) => {
                        assert_eq!(
                            result,
                            Err(PairingRejection::GamutClipped),
                            "{primaries:?} on {space:?}"
                        );
                    }
                    (epaint::Primaries::DisplayP3, Scs::Auto) => {
                        assert_eq!(
                            result,
                            Err(PairingRejection::UnresolvedSurfaceSpace),
                            "{primaries:?} on {space:?}"
                        );
                    }
                    (epaint::Primaries::DisplayP3, Scs::ExtendedSrgbLinear) => {
                        assert_eq!(
                            result,
                            Err(PairingRejection::LinearStorageNotPairedYet),
                            "{primaries:?} on {space:?}"
                        );
                    }
                    (_, Scs::Bt2100Pq | Scs::Bt2100Hlg) => {
                        assert_eq!(
                            result,
                            Err(PairingRejection::GamutClipped),
                            "{primaries:?} on {space:?}"
                        );
                    }
                }
            }
            // Unset active space (None) pairs with Srgb semantics.
            assert_eq!(
                pair_float_content_with_surface(primaries, None),
                pair_float_content_with_surface(primaries, Some(Scs::Srgb)),
                "None must pair like Srgb"
            );
        }
    }

    /// `OutputEncoding::of` is total over every reported color space × a
    /// representative format set, and matches plan §3's table (the normative
    /// rows): u8 raw views are `GammaEncoded` everywhere; `*Srgb` formats are
    /// `Linear` (hardware encodes — the row today's `is_srgb()` sniff
    /// selected, kept); the fp16 class is `Linear` exactly on the scRGB
    /// pairing and `GammaEncoded` everywhere else (`Srgb` included — legal
    /// in wgpu, so the table must cover it). Pure function — runs everywhere.
    #[test]
    fn output_encoding_of_is_total_and_matches_plan_table() {
        use wgpu::SurfaceColorSpace as Scs;
        use wgpu::TextureFormat as Tf;

        let spaces = [
            Scs::Auto,
            Scs::Srgb,
            Scs::ExtendedSrgb,
            Scs::ExtendedSrgbLinear,
            Scs::DisplayP3,
            Scs::ExtendedDisplayP3,
            Scs::Bt2100Pq,
            Scs::Bt2100Hlg,
        ];

        for space in spaces {
            // u8 raw views: gamma-encoded on every space (§3 rows 1–2).
            for format in [Tf::Bgra8Unorm, Tf::Rgba8Unorm] {
                assert_eq!(
                    OutputEncoding::of(format, space),
                    OutputEncoding::GammaEncoded,
                    "{format:?} + {space:?}"
                );
            }
            // `*Srgb` formats: hardware encodes, so the fragment writes
            // linear on every space (the kept `is_srgb()` row).
            assert_eq!(
                OutputEncoding::of(Tf::Bgra8UnormSrgb, space),
                OutputEncoding::Linear,
                "Bgra8UnormSrgb + {space:?}"
            );
            // The fp16 class: linear only on the scRGB pairing.
            let expected = if space == Scs::ExtendedSrgbLinear {
                OutputEncoding::Linear
            } else {
                OutputEncoding::GammaEncoded
            };
            assert_eq!(
                OutputEncoding::of(Tf::Rgba16Float, space),
                expected,
                "Rgba16Float + {space:?}"
            );
            // Ungrantable combos read as `GammaEncoded` (the table's rows,
            // not a guess): a u8 surface is never paired with the linear
            // space, and no non-fp16 float target is grantable.
            assert_eq!(
                OutputEncoding::of(Tf::Bgra8Unorm, Scs::ExtendedSrgbLinear),
                OutputEncoding::GammaEncoded
            );
            assert_eq!(
                OutputEncoding::of(Tf::Rgba32Float, Scs::ExtendedSrgbLinear),
                OutputEncoding::GammaEncoded
            );
        }
    }

    /// Every f16-exact value used by the round-trip test: SDR white, >1.0,
    /// negatives, ±0.0 (sign bit), and f16-max. `width` is 3 on purpose:
    /// 3 × 8 = 24 bytes/row, forcing the 256-byte pitch padding (plan §4).
    fn roundtrip_float_image(primaries: epaint::Primaries) -> epaint::FloatImage {
        let texels: [[f32; 4]; 6] = [
            [1.0, 0.5, 0.25, 1.0],
            [2.0, -1.0, 4.0, 0.5],
            [10.0, -0.25, 0.0, 1.0],
            [65504.0, 0.5, -2.0, 1.0],
            [0.25, 0.25, 0.25, 0.25],
            [-4.0, 4.0, -0.0, 1.0],
        ];
        let mut rgba = Vec::with_capacity(24);
        for texel in &texels {
            rgba.extend_from_slice(texel);
        }
        epaint::FloatImage::from_rgba([3, 2], primaries, &rgba)
    }

    /// The expected f16 bit patterns of [`roundtrip_float_image`], row-major.
    fn roundtrip_expected_bits() -> Vec<[u16; 4]> {
        roundtrip_float_image(epaint::Primaries::Srgb)
            .pixels()
            .iter()
            .map(|texel| texel.map(half::f16::to_bits))
            .collect()
    }

    /// Reads back an `Rgba16Float` texture as exact `u16` texel bit patterns.
    fn read_rgba16float_bits(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
    ) -> Vec<[u16; 4]> {
        let bytes_per_row = (width * 8).div_ceil(256) * 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("egui-wgpu float upload round-trip readback"),
            size: (bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("egui-wgpu float upload round-trip copy"),
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(10)),
            })
            .expect("GPU poll failed while waiting for the float upload copy");

        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).expect("readback receiver hung up");
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(10)),
            })
            .expect("GPU poll failed while waiting for the float upload map");
        receiver
            .recv()
            .expect("readback sender hung up")
            .expect("failed to map the float upload readback buffer");

        let data = slice
            .get_mapped_range()
            .expect("failed to map the float upload readback buffer");
        let mut texels = Vec::with_capacity((width * height) as usize);
        for row in 0..height {
            let row_start = (row * bytes_per_row) as usize;
            for col in 0..width {
                let offset = row_start + (col * 8) as usize;
                let texel: [u16; 4] = bytemuck::cast_slice(&data[offset..offset + 8])
                    .try_into()
                    .expect("each f16 texel is exactly eight bytes");
                texels.push(texel);
            }
        }
        texels
    }

    /// Float content uploads as `Rgba16Float` with f16-exact bits, including
    /// a width whose row pitch forces the 256-byte padding, and a
    /// `Primaries::DisplayP3` native pairing. Adapter-requiring (2a
    /// conventions): machines without any wgpu adapter skip with a message.
    #[test]
    fn float_upload_roundtrips_f16_bits_exactly() {
        let Some((device, queue)) = test_device("float_upload_roundtrips_f16_bits_exactly") else {
            return;
        };

        let mut renderer = Renderer::new(&device, RendererOptions::default());

        // Srgb-tagged content, sRGB active space: native.
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::Srgb));
        let image = roundtrip_float_image(epaint::Primaries::Srgb);
        let id = epaint::TextureId::Managed(101);
        let delta = epaint::ImageDelta::full(image, epaint::textures::TextureOptions::LINEAR);
        renderer.update_texture(&device, &queue, id, &delta);

        let texture = renderer
            .texture(&id)
            .and_then(|t| t.texture.as_ref())
            .expect("Srgb float content on an sRGB surface must allocate a texture");
        assert_eq!(
            texture.format(),
            wgpu::TextureFormat::Rgba16Float,
            "float content must upload as Rgba16Float"
        );
        assert_eq!(
            read_rgba16float_bits(&device, &queue, texture, 3, 2),
            roundtrip_expected_bits(),
            "the uploaded texels must be f16-bit-exact (padded 3-wide rows included)"
        );

        // DisplayP3-tagged content, DisplayP3 active space: native.
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::DisplayP3));
        let p3_image = roundtrip_float_image(epaint::Primaries::DisplayP3);
        let p3_id = epaint::TextureId::Managed(102);
        let p3_delta = epaint::ImageDelta::full(p3_image, epaint::textures::TextureOptions::LINEAR);
        renderer.update_texture(&device, &queue, p3_id, &p3_delta);

        let p3_texture = renderer
            .texture(&p3_id)
            .and_then(|t| t.texture.as_ref())
            .expect("DisplayP3 content on a DisplayP3 surface must allocate a texture");
        assert_eq!(p3_texture.format(), wgpu::TextureFormat::Rgba16Float);
        assert_eq!(
            read_rgba16float_bits(&device, &queue, p3_texture, 3, 2),
            roundtrip_expected_bits(),
            "the native DisplayP3 upload must also be f16-bit-exact"
        );

        // DisplayP3-tagged content on the EXTENDED P3 space — the pairing
        // `WideGamutHdr` grants on EDR displays — must upload native and
        // bit-exact too: the plain space above does not exercise the
        // extended arm of the pairing row.
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::ExtendedDisplayP3));
        let p3ext_id = epaint::TextureId::Managed(104);
        let p3ext_image = roundtrip_float_image(epaint::Primaries::DisplayP3);
        let p3ext_delta =
            epaint::ImageDelta::full(p3ext_image, epaint::textures::TextureOptions::LINEAR);
        renderer.update_texture(&device, &queue, p3ext_id, &p3ext_delta);

        let p3ext_texture = renderer
            .texture(&p3ext_id)
            .and_then(|t| t.texture.as_ref())
            .expect("DisplayP3 content on an ExtendedDisplayP3 surface must allocate a texture");
        assert_eq!(p3ext_texture.format(), wgpu::TextureFormat::Rgba16Float);
        assert_eq!(
            read_rgba16float_bits(&device, &queue, p3ext_texture, 3, 2),
            roundtrip_expected_bits(),
            "the native ExtendedDisplayP3 upload must also be f16-bit-exact"
        );

        // sRGB-tagged content on the scRGB-linear space — what this pairing
        // grants on fp16-capable surfaces (the `Off` target and the HDR
        // preferences) — rides native the same way: the linear entry point
        // decodes the gamma-encoded values (§7), so the upload is
        // byte-identical and must be f16-bit-exact.
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::ExtendedSrgbLinear));
        let scrgb_id = epaint::TextureId::Managed(105);
        let scrgb_image = roundtrip_float_image(epaint::Primaries::Srgb);
        let scrgb_delta =
            epaint::ImageDelta::full(scrgb_image, epaint::textures::TextureOptions::LINEAR);
        renderer.update_texture(&device, &queue, scrgb_id, &scrgb_delta);

        let scrgb_texture = renderer
            .texture(&scrgb_id)
            .and_then(|t| t.texture.as_ref())
            .expect("sRGB content on an ExtendedSrgbLinear surface must allocate a texture");
        assert_eq!(scrgb_texture.format(), wgpu::TextureFormat::Rgba16Float);
        assert_eq!(
            read_rgba16float_bits(&device, &queue, scrgb_texture, 3, 2),
            roundtrip_expected_bits(),
            "the native ExtendedSrgbLinear upload must also be f16-bit-exact"
        );

        // Partial (sub-region) float updates ride the same padded path: a
        // 3×2 patch written into a 3×4 texture (width 3 keeps forcing the
        // 256-byte pitch), then verified region-exact.
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::Srgb));
        let patch_id = epaint::TextureId::Managed(103);
        let base =
            epaint::FloatImage::filled([3, 4], epaint::Primaries::Srgb, [0.5, 0.5, 0.5, 1.0]);
        renderer.update_texture(
            &device,
            &queue,
            patch_id,
            &epaint::ImageDelta::full(base, epaint::textures::TextureOptions::LINEAR),
        );
        let patch = roundtrip_float_image(epaint::Primaries::Srgb);
        renderer.update_texture(
            &device,
            &queue,
            patch_id,
            &epaint::ImageDelta::partial([0, 2], patch, epaint::textures::TextureOptions::LINEAR),
        );
        let patched_texture = renderer
            .texture(&patch_id)
            .and_then(|t| t.texture.as_ref())
            .expect("the patched texture must exist");
        assert_eq!(patched_texture.format(), wgpu::TextureFormat::Rgba16Float);
        let patched_bits = read_rgba16float_bits(&device, &queue, patched_texture, 3, 4);
        let expected = roundtrip_expected_bits();
        let base_texel = [
            half::f16::from_f32(0.5).to_bits(),
            half::f16::from_f32(0.5).to_bits(),
            half::f16::from_f32(0.5).to_bits(),
            half::f16::from_f32(1.0).to_bits(),
        ];
        // 3-wide: rows are texels 0..3, 3..6, 6..9, 9..12; the patch is rows 2+3.
        assert_eq!(
            &patched_bits[0..6],
            &[base_texel; 6],
            "rows above the patch are untouched"
        );
        assert_eq!(
            &patched_bits[6..12],
            &expected[..],
            "the patch region is f16-bit-exact"
        );
        assert!(
            renderer.upload_rejections_warned.is_empty(),
            "no pairing rejection may fire in this test"
        );
    }

    /// DisplayP3-tagged float content on an sRGB surface is loudly rejected:
    /// no texture is created, a warning logs once per texture id, and the
    /// gate does not poison other uploads. Adapter-requiring (the reject
    /// path returns before touching the device, but `update_texture` takes
    /// device/queue references).
    #[test]
    fn display_p3_float_content_is_loudly_rejected_on_srgb_surface() {
        let Some((device, queue)) =
            test_device("display_p3_float_content_is_loudly_rejected_on_srgb_surface")
        else {
            return;
        };

        let mut renderer = Renderer::new(&device, RendererOptions::default());
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::Srgb));

        let p3_id = epaint::TextureId::Managed(201);
        let p3_delta = epaint::ImageDelta::full(
            roundtrip_float_image(epaint::Primaries::DisplayP3),
            epaint::textures::TextureOptions::LINEAR,
        );
        renderer.update_texture(&device, &queue, p3_id, &p3_delta);

        // The texture was NOT created/updated — observable via the map.
        assert!(
            !renderer.textures.contains_key(&p3_id),
            "a rejected upload must not allocate a texture"
        );
        // The honest warn assertion: the warn-once state (the log::warn! ran).
        assert!(
            renderer.upload_rejections_warned.contains(&p3_id),
            "a rejected upload must warn (once) for its texture id"
        );
        // Rejecting is not sticky: the same id with Srgb content uploads.
        let srgb_delta = epaint::ImageDelta::full(
            roundtrip_float_image(epaint::Primaries::Srgb),
            epaint::textures::TextureOptions::LINEAR,
        );
        renderer.update_texture(&device, &queue, p3_id, &srgb_delta);
        assert!(
            renderer.texture(&p3_id).is_some(),
            "the gate must not poison later, pairable uploads for the same id"
        );

        // The scRGB pairing since PR 4: sRGB-tagged content is native there
        // (the linear pipelines decode it) and uploads f16-bit-exact…
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::ExtendedSrgbLinear));
        let srgb_id_scrgb = epaint::TextureId::Managed(202);
        renderer.update_texture(&device, &queue, srgb_id_scrgb, &srgb_delta);
        let scrgb_texture = renderer
            .texture(&srgb_id_scrgb)
            .and_then(|t| t.texture.as_ref())
            .expect("Srgb float content on the scRGB pairing must upload natively");
        assert_eq!(scrgb_texture.format(), wgpu::TextureFormat::Rgba16Float);
        assert_eq!(
            read_rgba16float_bits(&device, &queue, scrgb_texture, 3, 2),
            roundtrip_expected_bits(),
            "the native scRGB upload must also be f16-bit-exact"
        );
        // …while DisplayP3 content still loudly rejects there (the primaries
        // conversion ships with §5's matrices).
        let p3_id_scrgb = epaint::TextureId::Managed(203);
        renderer.update_texture(&device, &queue, p3_id_scrgb, &p3_delta);
        assert!(
            renderer.texture(&p3_id_scrgb).is_none(),
            "DisplayP3 content on the linear pairing must stay rejected"
        );
        assert!(
            renderer.upload_rejections_warned.contains(&p3_id_scrgb),
            "the rejected scRGB pairing must warn (once) for its texture id"
        );
    }

    /// A partial patch whose content kind does not match the texture's
    /// format is loudly rejected in both directions: a Color patch onto a
    /// float texture (would fail wgpu's size validation) and a float patch
    /// onto an 8-bit texture (would silently corrupt). The patch is skipped,
    /// the previous content is untouched, and no device error fires
    /// (the `on_uncaptured_error` panic in [`test_device`] fails the test on
    /// any validation error). Adapter-requiring (2a conventions).
    #[test]
    fn partial_patch_kind_mismatch_is_loudly_rejected() {
        let Some((device, queue)) = test_device("partial_patch_kind_mismatch_is_loudly_rejected")
        else {
            return;
        };

        let mut renderer = Renderer::new(&device, RendererOptions::default());
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::Srgb));

        // Float texture, then a Color patch onto it: skipped, texture
        // bit-identical.
        let float_id = epaint::TextureId::Managed(301);
        renderer.update_texture(
            &device,
            &queue,
            float_id,
            &epaint::ImageDelta::full(
                roundtrip_float_image(epaint::Primaries::Srgb),
                epaint::textures::TextureOptions::LINEAR,
            ),
        );
        let texture = renderer
            .texture(&float_id)
            .and_then(|t| t.texture.as_ref())
            .expect("the float texture must exist");
        let before = read_rgba16float_bits(&device, &queue, texture, 3, 2);

        renderer.update_texture(
            &device,
            &queue,
            float_id,
            &epaint::ImageDelta::partial(
                [0, 0],
                epaint::ColorImage::filled([1, 2], epaint::Color32::RED),
                epaint::textures::TextureOptions::LINEAR,
            ),
        );
        assert!(
            renderer.upload_rejections_warned.contains(&float_id),
            "a Color patch onto a float texture must warn (once)"
        );
        let texture = renderer
            .texture(&float_id)
            .and_then(|t| t.texture.as_ref())
            .expect("the skipped patch must not remove the texture");
        assert_eq!(texture.format(), wgpu::TextureFormat::Rgba16Float);
        assert_eq!(
            read_rgba16float_bits(&device, &queue, texture, 3, 2),
            before,
            "the skipped patch must leave the texture bit-identical"
        );

        // Color texture, then a Float patch onto it: skipped, format and
        // content unchanged.
        let color_id = epaint::TextureId::Managed(302);
        renderer.update_texture(
            &device,
            &queue,
            color_id,
            &epaint::ImageDelta::full(
                epaint::ColorImage::filled([3, 2], epaint::Color32::RED),
                epaint::textures::TextureOptions::LINEAR,
            ),
        );
        renderer.update_texture(
            &device,
            &queue,
            color_id,
            &epaint::ImageDelta::partial(
                [0, 0],
                roundtrip_float_image(epaint::Primaries::Srgb),
                epaint::textures::TextureOptions::LINEAR,
            ),
        );
        assert!(
            renderer.upload_rejections_warned.contains(&color_id),
            "a float patch onto a Color texture must warn (once)"
        );
        let texture = renderer
            .texture(&color_id)
            .and_then(|t| t.texture.as_ref())
            .expect("the skipped patch must not remove the texture");
        assert_eq!(
            texture.format(),
            wgpu::TextureFormat::Rgba8Unorm,
            "the skipped patch must not change the texture's format"
        );

        // The same kind re-uploaded whole still works after a mismatch (the
        // rejection is not sticky for valid uploads).
        renderer.update_texture(
            &device,
            &queue,
            color_id,
            &epaint::ImageDelta::full(
                epaint::ColorImage::filled([3, 2], epaint::Color32::BLUE),
                epaint::textures::TextureOptions::LINEAR,
            ),
        );
        assert!(
            renderer.texture(&color_id).is_some(),
            "a whole upload of the matching kind must succeed after a mismatch"
        );
    }

    /// Adapter/device bootstrap shared by the runtime tests: skips with a
    /// message on machines without any wgpu adapter (2a conventions).
    fn test_device(context: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })) {
                Ok(adapter) => adapter,
                Err(err) => {
                    // Machines without any wgpu adapter skip; CI (macos) and dev
                    // machines with GPUs (or software rasterizers) always run this.
                    #[expect(clippy::print_stderr)]
                    {
                        eprintln!("skipping {context}: no wgpu adapter: {err}");
                    }
                    return None;
                }
            };

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some(context),
            ..Default::default()
        }))
        .unwrap_or_else(|err| panic!("failed to request a wgpu device for {context}: {err}"));

        // Any device error (validation or otherwise) fails the test.
        device.on_uncaptured_error(Arc::new(|err| {
            panic!("wgpu device error during the egui-wgpu runtime test: {err}");
        }));

        Some((device, queue))
    }

    // -----------------------------------------------------------------------
    // PR 3 — fp16 extended-range pipelines (plan §8 PR 3 test matrix).
    //
    // All render through the PRODUCTION `Renderer` (pipeline cache, uniform
    // path, dither module choice) onto offscreen `Rgba16Float` targets, and
    // read back through the shared `crate::readback` decoder. The content is
    // gamma-encoded contract-layout `FloatImage` (plan §4), so the CPU-side
    // expectations are exact up to the one f16 quantization the content type
    // performs.
    // -----------------------------------------------------------------------

    /// Screen size of the one-quad scenes below: 16×2 px, so the 16-texel
    /// content row maps 1:1 and the row pitch (16 × 8 B = 128) forces the
    /// 256-byte padding.
    const HDR_SCENE_W: u32 = 16;
    const HDR_SCENE_H: u32 = 2;

    /// The gamma-encoded content of the numeric scene: one texel per column,
    /// both rows identical. Layout per plan §4: premultiplied in the encoded
    /// components (alpha 1 ⇒ identity), unclamped above 1.0, odd-symmetric
    /// for negatives.
    fn numeric_scene_texels() -> Vec<[f32; 4]> {
        let values: [f32; HDR_SCENE_W as usize] = [
            epaint::ecolor::gamma_from_linear(0.25),
            epaint::ecolor::gamma_from_linear(0.5),
            epaint::ecolor::gamma_from_linear(0.75),
            epaint::ecolor::gamma_from_linear(0.04), // the toe region
            epaint::ecolor::gamma_from_linear(0.003_130_8), // the encode knee
            epaint::ecolor::gamma_from_linear(1e-6), // encodes into f16 subnormals
            1.0,                                     // SDR reference white
            2.0,                                     // the classic HDR headroom value
            10.0,                                    // deep headroom
            65504.0, // f16 max: the value must not round-trip to inf
            -1.0,
            -0.5,
            -4.0,
            0.0,
            -0.0,
            1e-5, // a positive f16-subnormal
        ];
        values
            .iter()
            .chain(values.iter()) // both scene rows identical, row-major
            .map(|&v| [v, v, v, 1.0])
            .collect()
    }

    /// One full-viewport quad textured with `texture_id`, in the layout
    /// `update_buffers` consumes (the UV grid maps the quad onto a
    /// `HDR_SCENE_W × 1` content image).
    fn textured_quad(texture_id: epaint::TextureId) -> epaint::Mesh {
        let [sw, sh] = [f32::from(HDR_SCENE_W as u16), f32::from(HDR_SCENE_H as u16)];
        let uv = |x: f32, y: f32| epaint::Pos2::new(x / HDR_SCENE_W as f32, y);
        let vertices = [
            (epaint::Pos2::ZERO, uv(0.0, 0.0)),
            (epaint::Pos2::new(sw, 0.0), uv(HDR_SCENE_W as f32, 0.0)),
            (epaint::Pos2::new(sw, sh), uv(HDR_SCENE_W as f32, 1.0)),
            (epaint::Pos2::new(0.0, sh), uv(0.0, 1.0)),
        ]
        .map(|(pos, uv)| epaint::Vertex {
            pos,
            uv,
            color: epaint::Color32::WHITE,
        });
        epaint::Mesh {
            indices: vec![0, 1, 2, 0, 2, 3],
            vertices: vertices.to_vec(),
            texture_id,
        }
    }

    /// Renders one mesh through the production `Renderer` onto an offscreen
    /// target (`msaa_samples` > 1 goes through an intermediate multi-sampled
    /// attachment resolved into the target — the plumbing deliverable) and
    /// returns the copied readback buffer, ready for
    /// [`crate::readback::texture_to_float_image`] (float formats) or the u8
    /// decode (8-bit formats).
    fn render_scene(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut Renderer,
        target_format: wgpu::TextureFormat,
        msaa_samples: u32,
        mesh: epaint::Mesh,
    ) -> wgpu::Buffer {
        let screen = ScreenDescriptor {
            size_in_pixels: [HDR_SCENE_W, HDR_SCENE_H],
            pixels_per_point: 1.0,
        };
        let clip = epaint::ClippedPrimitive {
            clip_rect: epaint::Rect::from_min_size(
                epaint::Pos2::ZERO,
                epaint::vec2(HDR_SCENE_W as f32, HDR_SCENE_H as f32),
            ),
            primitive: epaint::Primitive::Mesh(mesh),
        };
        let primitives = [clip];

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        let user_cmd_bufs = renderer.update_buffers(
            device,
            queue,
            &mut encoder,
            &primitives,
            &screen,
            target_format,
        );

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pr3 scene target"),
            size: wgpu::Extent3d {
                width: HDR_SCENE_W,
                height: HDR_SCENE_H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: target_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

        let (view, resolve_target) = if msaa_samples > 1 {
            let intermediate = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("pr3 scene msaa intermediate"),
                size: wgpu::Extent3d {
                    width: HDR_SCENE_W,
                    height: HDR_SCENE_H,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: msaa_samples,
                dimension: wgpu::TextureDimension::D2,
                format: target_format, // §9: the intermediate uses the target format
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            (
                intermediate.create_view(&wgpu::TextureViewDescriptor::default()),
                Some(&target_view),
            )
        } else {
            (target_view.clone(), None)
        };

        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("pr3 scene pass"),
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
            renderer.render(&mut pass, &primitives, &screen, target_format);
        }

        queue.submit(core::iter::chain(user_cmd_bufs, [encoder.finish()]));

        // Copy the target into a pitch-padded buffer (256-byte alignment).
        // The scenes only use the two uncompressed RGBA targets.
        let bytes_per_pixel = match target_format {
            wgpu::TextureFormat::Rgba16Float => 8,
            wgpu::TextureFormat::Rgba8Unorm => 4,
            other => {
                unreachable!("the scenes only render to Rgba8Unorm/Rgba16Float, got {other:?}")
            }
        };
        let bytes_per_row = wgpu::util::align_to(
            HDR_SCENE_W * bytes_per_pixel,
            wgpu::COPY_BYTES_PER_ROW_ALIGNMENT,
        );
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pr3 scene readback"),
            size: u64::from(bytes_per_row * HDR_SCENE_H),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: HDR_SCENE_W,
                height: HDR_SCENE_H,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        readback
    }

    /// Reads back a pitch-padded `Rgba8Unorm` copy as float texels in
    /// `[0, 1]` (premultiplied), matching [`crate::readback`]'s shape.
    fn read_u8_pixels(
        device: &wgpu::Device,
        buffer: &wgpu::Buffer,
        size: [u32; 2],
        bytes_per_row: u32,
    ) -> Vec<[f32; 4]> {
        assert_eq!(
            u64::from(bytes_per_row * size[1]),
            buffer.size(),
            "readback buffer size must match size × bytes_per_row"
        );
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).expect("readback receiver hung up");
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(10)),
            })
            .expect("GPU poll failed while waiting for the u8 readback map");
        receiver
            .recv()
            .expect("readback sender hung up")
            .expect("failed to map the u8 readback buffer");
        let data = slice
            .get_mapped_range()
            .expect("failed to map the u8 readback buffer");
        let mut texels = Vec::with_capacity((size[0] * size[1]) as usize);
        for row in 0..size[1] {
            let row_start = (row * bytes_per_row) as usize;
            for col in 0..size[0] {
                let offset = row_start + (col * 4) as usize;
                let texel: [u8; 4] = data[offset..offset + 4]
                    .try_into()
                    .expect("each RGBA8 texel is exactly four bytes");
                texels.push(texel.map(f32::from).map(|v| v / 255.0));
            }
        }
        texels
    }

    /// The expected readback of `numeric_scene_texels` content: the CPU f16
    /// quantization the content type performs (round-to-nearest). The path is
    /// value-exact beyond that — zero-copy upload, exact 1.0× multiply in the
    /// shader, exact `a=1` premultiplied blend, f16-exact store — so these
    /// are compared for equality, not within a tolerance.
    fn expected_numeric_texels() -> Vec<[f32; 4]> {
        numeric_scene_texels()
            .iter()
            .map(|texel| texel.map(half::f16::from_f32).map(half::f16::to_f32))
            .collect()
    }

    /// The §8 PR 3 numeric matrix: known-linear rows surface as
    /// `gamma_from_linear` within f16 quantization (±2⁻¹¹ relative in the
    /// normal range, subnormal carve-out); 2.0, 10.0 and near-f16-max survive
    /// upload→shader→blend; negatives stay odd-symmetric. Adapter-requiring
    /// (2a conventions).
    #[test]
    fn fp16_extended_surface_preserves_contract_values() {
        let Some((device, queue)) = test_device("fp16_extended_surface_preserves_contract_values")
        else {
            return;
        };

        let mut renderer = Renderer::new(&device, RendererOptions::default());
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::Srgb));
        let texels = numeric_scene_texels();
        let flat: Vec<f32> = texels.iter().flat_map(|t| *t).collect();
        let image = epaint::FloatImage::from_rgba(
            [HDR_SCENE_W as usize, HDR_SCENE_H as usize],
            epaint::Primaries::Srgb,
            &flat,
        );
        let id = epaint::TextureId::Managed(401);
        renderer.update_texture(
            &device,
            &queue,
            id,
            &epaint::ImageDelta::full(image, epaint::textures::TextureOptions::NEAREST),
        );
        assert!(
            renderer.texture(&id).is_some(),
            "Srgb-tagged content on an sRGB surface must upload"
        );

        let readback = render_scene(
            &device,
            &queue,
            &mut renderer,
            wgpu::TextureFormat::Rgba16Float,
            1,
            textured_quad(id),
        );
        let pixels = crate::readback::texture_to_float_image(
            &device,
            &readback,
            [HDR_SCENE_W, HDR_SCENE_H],
            wgpu::util::align_to(HDR_SCENE_W * 8, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
        );

        let expected = expected_numeric_texels();
        let inputs = numeric_scene_texels();
        for (i, pixel) in pixels.iter().enumerate() {
            let want = expected[i];
            let raw = inputs[i % HDR_SCENE_W as usize];
            // Exactness of the pass-through (the contract's stored value).
            assert_eq!(*pixel, want, "texel {i}: exact f16 pass-through");
            // The §8 numeric bound for the gamma-encoded rows, stated against
            // the unquantized value: ±2⁻¹¹ relative (half an f16 step) in the
            // normal range, an absolute carve-out in the subnormal range.
            for lane in 0..3 {
                let (gpu, cpu) = (pixel[lane], raw[lane]);
                if cpu.abs() < 6.103_515_6e-5 {
                    assert!(
                        (gpu - cpu).abs() <= 1.192_092_9e-7,
                        "texel {i} lane {lane}: subnormal {cpu} -> {gpu}"
                    );
                } else if cpu != 0.0 {
                    assert!(
                        !gpu.is_nan() && (gpu - cpu).abs() <= cpu.abs() * 4.882_812_5e-4,
                        "texel {i} lane {lane}: {cpu} -> {gpu} (±2⁻¹¹ relative)"
                    );
                }
            }
            assert_eq!(pixel[3], 1.0, "texel {i}: alpha stays 1.0");
        }
    }

    /// Dithering is structural per target class (plan §3): with the knob ON,
    /// the u8 target dithers (frame differs from the knob-OFF frame — today's
    /// behavior, regression guard for the gate), and the fp16 target does NOT
    /// (knob-ON frame bit-identical to the knob-OFF frame, and two repeats
    /// bit-identical). The probe is a bilinear gradient — vertex colors sit
    /// exactly on 8-bit codes, which is precisely why the knob only ever
    /// mattered for interpolated values. Adapter-requiring.
    #[test]
    fn dither_applies_on_u8_and_never_on_fp16() {
        let Some((device, queue)) = test_device("dither_applies_on_u8_and_never_on_fp16") else {
            return;
        };

        // Two renderers of the same scene, differing only in the knob.
        let mut dither_on = Renderer::new(&device, RendererOptions::default());
        let mut dither_off = Renderer::new(&device, RendererOptions::PREDICTABLE);
        dither_on.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::Srgb));
        dither_off.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::Srgb));

        // A 2-texel gradient, LINEAR-filtered, stretched over the viewport:
        // every pixel interpolates to a value between 8-bit codes.
        let gradient = || {
            epaint::FloatImage::from_rgba(
                [2, 1],
                epaint::Primaries::Srgb,
                &[0.25, 0.25, 0.25, 1.0, 0.75, 0.75, 0.75, 1.0],
            )
        };
        let scene = |renderer: &mut Renderer, id: epaint::TextureId| {
            renderer.update_texture(
                &device,
                &queue,
                id,
                &epaint::ImageDelta::full(gradient(), epaint::textures::TextureOptions::LINEAR),
            );
        };
        let (on_id, off_id) = (
            epaint::TextureId::Managed(402),
            epaint::TextureId::Managed(403),
        );
        scene(&mut dither_on, on_id);
        scene(&mut dither_off, off_id);

        let fp16_pitch = wgpu::util::align_to(HDR_SCENE_W * 8, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let u8_pitch = wgpu::util::align_to(HDR_SCENE_W * 4, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let fp16 = |renderer: &mut Renderer, id| {
            let readback = render_scene(
                &device,
                &queue,
                renderer,
                wgpu::TextureFormat::Rgba16Float,
                1,
                textured_quad(id),
            );
            crate::readback::texture_to_float_image(
                &device,
                &readback,
                [HDR_SCENE_W, HDR_SCENE_H],
                fp16_pitch,
            )
        };
        let u8 = |renderer: &mut Renderer, id| {
            let readback = render_scene(
                &device,
                &queue,
                renderer,
                wgpu::TextureFormat::Rgba8Unorm,
                1,
                textured_quad(id),
            );
            read_u8_pixels(&device, &readback, [HDR_SCENE_W, HDR_SCENE_H], u8_pitch)
        };

        let fp16_on = fp16(&mut dither_on, on_id);
        let fp16_on_repeat = fp16(&mut dither_on, on_id);
        let fp16_off = fp16(&mut dither_off, off_id);
        let u8_on = u8(&mut dither_on, on_id);
        let u8_off = u8(&mut dither_off, off_id);

        // Two frames of the same fp16 scene: bit-identical.
        assert_eq!(fp16_on, fp16_on_repeat, "two frames of the same fp16 scene");

        // The knob cannot reach fp16: forcing it off changes nothing.
        assert_eq!(
            fp16_on, fp16_off,
            "fp16 frames must be identical with dithering on and off"
        );

        // The knob still works on u8: at least one pixel dithers to a
        // different code.
        assert_ne!(
            u8_on, u8_off,
            "the u8 path must still dither with the knob on"
        );
    }

    /// A [0,1] SDR scene through the fp16 path matches the u8-path output
    /// within tolerance (§2: ≤ ½-code drift where the u8 path quantized at
    /// store; the bound absorbs blend-precision drift between targets).
    /// Adapter-requiring.
    #[test]
    fn sdr_scene_matches_u8_path_on_fp16() {
        let Some((device, queue)) = test_device("sdr_scene_matches_u8_path_on_fp16") else {
            return;
        };

        let mut renderer = Renderer::new(&device, RendererOptions::PREDICTABLE);
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::Srgb));
        let sdr_values: Vec<[f32; 4]> = (0..HDR_SCENE_W)
            .map(|x| {
                let v = epaint::ecolor::gamma_from_linear(x as f32 / (HDR_SCENE_W - 1) as f32);
                [v, v, v, 1.0]
            })
            .collect();
        let sdr_flat: Vec<f32> = sdr_values.iter().flat_map(|t| *t).collect();
        let image = epaint::FloatImage::from_rgba(
            [HDR_SCENE_W as usize, 1],
            epaint::Primaries::Srgb,
            &sdr_flat,
        );
        let id = epaint::TextureId::Managed(403);
        renderer.update_texture(
            &device,
            &queue,
            id,
            &epaint::ImageDelta::full(image, epaint::textures::TextureOptions::NEAREST),
        );
        let quad = textured_quad(id);

        let fp16_readback = render_scene(
            &device,
            &queue,
            &mut renderer,
            wgpu::TextureFormat::Rgba16Float,
            1,
            quad.clone(),
        );
        let fp16 = crate::readback::texture_to_float_image(
            &device,
            &fp16_readback,
            [HDR_SCENE_W, HDR_SCENE_H],
            wgpu::util::align_to(HDR_SCENE_W * 8, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
        );
        let u8_readback = render_scene(
            &device,
            &queue,
            &mut renderer,
            wgpu::TextureFormat::Rgba8Unorm,
            1,
            quad,
        );
        let u8 = read_u8_pixels(
            &device,
            &u8_readback,
            [HDR_SCENE_W, HDR_SCENE_H],
            wgpu::util::align_to(HDR_SCENE_W * 4, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
        );

        assert_eq!(fp16.len(), u8.len());
        for (i, (f, u)) in fp16.iter().zip(u8.iter()).enumerate() {
            for lane in 0..4 {
                assert!(
                    (f[lane] - u[lane]).abs() <= 2.0 / 255.0,
                    "texel {i} lane {lane}: fp16 {f:?} vs u8 {u:?} drifted beyond a code"
                );
            }
        }
    }

    /// MSAA on fp16 resolves (§9): the multi-sampled intermediate (same
    /// format as the target) resolves into the single-sampled target and the
    /// content survives. Skips with a message — never silent-pass — when the
    /// adapter cannot resolve `Rgba16Float` (§9's apple1x caveat).
    /// Adapter-requiring.
    #[test]
    fn msaa_fp16_resolves_content() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })) {
                Ok(adapter) => adapter,
                Err(err) => {
                    #[expect(clippy::print_stderr)]
                    {
                        eprintln!("skipping msaa_fp16_resolves_content: no wgpu adapter: {err}");
                    }
                    return;
                }
            };
        let format_features = adapter.get_texture_format_features(wgpu::TextureFormat::Rgba16Float);
        if !format_features
            .flags
            .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
        {
            #[expect(clippy::print_stderr)]
            {
                eprintln!(
                    "skipping msaa_fp16_resolves_content: adapter ({:?}) cannot resolve Rgba16Float (§9 apple1x caveat)",
                    adapter.get_info().name
                );
            }
            return;
        }

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("msaa fp16 resolve test"),
            ..Default::default()
        }))
        .expect("failed to request a wgpu device for the msaa fp16 resolve test");
        device.on_uncaptured_error(Arc::new(|err| {
            panic!("wgpu device error during the msaa fp16 resolve test: {err}");
        }));

        // The pipeline must be built for 4× MSAA (multisample.count).
        let mut renderer = Renderer::new(
            &device,
            RendererOptions {
                msaa_samples: 4,
                ..RendererOptions::PREDICTABLE
            },
        );
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::Srgb));
        let msaa_values: Vec<[f32; 4]> = (0..HDR_SCENE_W)
            .map(|x| {
                let v = 1.0 + x as f32; // HDR: 1..17, all above SDR white
                [v, v, v, 1.0]
            })
            .collect();
        let msaa_flat: Vec<f32> = msaa_values.iter().flat_map(|t| *t).collect();
        let image = epaint::FloatImage::from_rgba(
            [HDR_SCENE_W as usize, 1],
            epaint::Primaries::Srgb,
            &msaa_flat,
        );
        let id = epaint::TextureId::Managed(404);
        renderer.update_texture(
            &device,
            &queue,
            id,
            &epaint::ImageDelta::full(image, epaint::textures::TextureOptions::NEAREST),
        );

        let readback = render_scene(
            &device,
            &queue,
            &mut renderer,
            wgpu::TextureFormat::Rgba16Float,
            4,
            textured_quad(id),
        );
        let pixels = crate::readback::texture_to_float_image(
            &device,
            &readback,
            [HDR_SCENE_W, HDR_SCENE_H],
            wgpu::util::align_to(HDR_SCENE_W * 8, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
        );

        assert_eq!(pixels.len(), (HDR_SCENE_W * HDR_SCENE_H) as usize);
        for (i, pixel) in pixels.iter().enumerate() {
            let x = (i % HDR_SCENE_W as usize) as f32;
            let want = half::f16::from_f32(1.0 + x).to_f32();
            assert_eq!(
                pixel[0], pixel[1],
                "texel {i}: resolved content must be uniform per texel"
            );
            for (lane, v) in pixel.iter().take(3).enumerate() {
                assert_eq!(*v, want, "texel {i} lane {lane}: resolved {want} damaged");
            }
        }
    }

    // -----------------------------------------------------------------------
    // PR 4 — the derived linear path (plan §3 + §7).
    // -----------------------------------------------------------------------

    /// The scale-math unit test (plan §7's anchor): the uniform value for the
    /// linear entry point is `sdr_white_nits / 80`, and the no-report default
    /// is exactly 1.0. Pure function — runs everywhere, no adapter needed.
    #[test]
    fn sdr_white_scale_math() {
        assert_eq!(sdr_white_scale(None), 1.0, "no report → the 80-nit anchor");
        // Also the anchor check: scale(nits) is 1.0 exactly when the constant
        // divides nits, so `Some(80.0) → 1.0` pins `SDR_REFERENCE_WHITE_NITS`
        // at the IEC 61966-2-1 reference white.
        assert_eq!(
            sdr_white_scale(Some(80.0)),
            1.0,
            "80 nits is the anchor: no rescaling"
        );
        assert_eq!(
            sdr_white_scale(Some(200.0)),
            2.5,
            "200 nits over the 80-nit anchor"
        );
    }

    /// The gamma-encoded scene for the linear-decode test: 16 columns, both
    /// rows identical, §4-contract values (unclamped, odd-symmetric, both
    /// zeros, subnormal decodes). It omits magnitudes whose *decoded* value
    /// leaves f16 range (`65504²·⁴` overflows; the passthrough matrix in
    /// [`Self::fp16_extended_surface_preserves_contract_values`] covers those
    /// on the gamma path, which never decodes).
    fn linear_scene_encoded_texels() -> Vec<[f32; 4]> {
        let encoded: [f32; HDR_SCENE_W as usize] = [
            epaint::ecolor::gamma_from_linear(0.25),
            epaint::ecolor::gamma_from_linear(0.5),
            epaint::ecolor::gamma_from_linear(0.75),
            epaint::ecolor::gamma_from_linear(0.04), // the decode toe
            epaint::ecolor::gamma_from_linear(0.003_130_8), // the encode knee
            epaint::ecolor::gamma_from_linear(1e-6), // subnormal decode
            1.0,                                     // SDR reference white
            2.0,                                     // classic headroom value
            10.0, // deep headroom, still f16-representable decoded
            -1.0,
            -0.5,
            -4.0, // odd-symmetric decode
            0.0,
            -0.0, // both zeros: decode must not NaN
            1e-5, // positive f16-subnormal decode
            1e-6,
        ];
        encoded
            .iter()
            .chain(encoded.iter()) // both scene rows identical, row-major
            .map(|&v| [v, v, v, 1.0])
            .collect()
    }

    /// Renders the linear scene once through the production `Renderer` onto
    /// the offscreen fp16 target and returns the decoded readback texels.
    fn render_linear_scene(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut Renderer,
        texture_id: epaint::TextureId,
    ) -> Vec<[f32; 4]> {
        let readback = render_scene(
            device,
            queue,
            renderer,
            wgpu::TextureFormat::Rgba16Float,
            1,
            textured_quad(texture_id),
        );
        crate::readback::texture_to_float_image(
            device,
            &readback,
            [HDR_SCENE_W, HDR_SCENE_H],
            wgpu::util::align_to(HDR_SCENE_W * 8, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
        )
    }

    /// The linear path end-to-end, headless (scRGB is never *reported* on
    /// macOS, but the encoding is a derived quantity, so the fp16 target is
    /// forced through `OutputEncoding::Linear` by the same plumbing a
    /// Windows surface drives: active space `ExtendedSrgbLinear` →
    /// [`OutputEncoding::of`] → the decode entry point). Asserts, against the
    /// shared 2a CPU references (`ecolor::linear_from_gamma`, negative inputs
    /// included):
    /// - scale 1.0 (default, 80 nits, no report): gamma-encoded in → linear
    ///   out, within the §8 f16 tolerance, alpha untouched, zeros intact;
    /// - 200 nits → ×2.5 (the §7 anchor math), back to ×1.0 at 80 nits/None;
    /// - the `GammaEncoded` entry point ignores the uniform entirely (exact
    ///   passthrough with nits set).
    ///
    /// Not covered here, honestly: a real scRGB *surface* round-trip — that
    /// needs Windows/DXGI (`MANUAL_TESTING.md`). Adapter-requiring (2a
    /// conventions).
    #[test]
    fn linear_encoding_decodes_and_scales_on_fp16_target() {
        let Some((device, queue)) =
            test_device("linear_encoding_decodes_and_scales_on_fp16_target")
        else {
            return;
        };

        let mut renderer = Renderer::new(&device, RendererOptions::default());
        // The scRGB pairing: (Srgb content, ExtendedSrgbLinear surface) has
        // been native since the PR 4 pairing flip.
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::ExtendedSrgbLinear));
        let inputs = linear_scene_encoded_texels();
        let flat: Vec<f32> = inputs.iter().flat_map(|t| *t).collect();
        let image = epaint::FloatImage::from_rgba(
            [HDR_SCENE_W as usize, HDR_SCENE_H as usize],
            epaint::Primaries::Srgb,
            &flat,
        );
        let id = epaint::TextureId::Managed(405);
        renderer.update_texture(
            &device,
            &queue,
            id,
            &epaint::ImageDelta::full(image, epaint::textures::TextureOptions::NEAREST),
        );
        assert!(
            renderer.texture(&id).is_some(),
            "Srgb content on the scRGB pairing must upload"
        );

        let assert_decoded = |pixels: &[[f32; 4]], scale: f32, context: &str| {
            assert_eq!(pixels.len(), (HDR_SCENE_W * HDR_SCENE_H) as usize);
            for (i, pixel) in pixels.iter().enumerate() {
                let encoded = inputs[i % HDR_SCENE_W as usize];
                for (lane, &v) in encoded[..3].iter().enumerate() {
                    let want = epaint::ecolor::linear_from_gamma(v) * scale;
                    assert_f16_close(
                        pixel[lane],
                        want,
                        &format!("{context}: texel {i} lane {lane} (encoded {v})"),
                    );
                }
                assert_eq!(pixel[3], 1.0, "{context}: texel {i}: alpha stays 1.0");
            }
        };

        // Scale 1.0 — the default — is the pure decode.
        let pixels = render_linear_scene(&device, &queue, &mut renderer, id);
        assert_decoded(&pixels, 1.0, "default scale");

        // The §7 anchor math: 200 nits → ×2.5; 80 nits and no report → ×1.0.
        renderer.set_sdr_white_nits(Some(200.0));
        let pixels = render_linear_scene(&device, &queue, &mut renderer, id);
        assert_decoded(&pixels, 2.5, "200-nit scale");

        renderer.set_sdr_white_nits(Some(80.0));
        let pixels = render_linear_scene(&device, &queue, &mut renderer, id);
        assert_decoded(&pixels, 1.0, "80-nit scale");

        renderer.set_sdr_white_nits(None);
        let pixels = render_linear_scene(&device, &queue, &mut renderer, id);
        assert_decoded(&pixels, 1.0, "no-report scale");

        // The GammaEncoded entry point never reads the uniform: same scene,
        // same (2.5×) nits, sRGB active space — exact f16 passthrough.
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::Srgb));
        renderer.set_sdr_white_nits(Some(200.0));
        let pixels = render_linear_scene(&device, &queue, &mut renderer, id);
        for (i, pixel) in pixels.iter().enumerate() {
            let encoded = inputs[i % HDR_SCENE_W as usize];
            let want = encoded.map(half::f16::from_f32).map(half::f16::to_f32);
            assert_eq!(
                *pixel, want,
                "texel {i}: the gamma path must ignore the SDR-white uniform"
            );
        }
    }
}
