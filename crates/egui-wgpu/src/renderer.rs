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
    /// Dithering assumes an sRGB output and thus will apply noise to any input value that lies between
    /// two 8bit values after applying the sRGB OETF function, i.e. if it's not a whole 8bit value in "gamma space".
    /// This means that only inputs from texture interpolation and vertex colors should be affected in practice.
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
    /// Render pipelines keyed by the format of the render-pass target.
    ///
    /// Created lazily on first use of each format, in
    /// [`Self::update_buffers`]. Only the target format bakes a pipeline:
    /// a color-space change is a surface reconfigure, never a pipeline change.
    pipelines: Mutex<HashMap<wgpu::TextureFormat, Arc<wgpu::RenderPipeline>>>,

    /// The egui shader module, from which all pipelines in the cache are built.
    shader_module: wgpu::ShaderModule,

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

    /// The color space of the surface this renderer currently presents to, as
    /// granted by the surface decision. Set by the painter (winit / web) each
    /// frame, under its renderer lock, before the per-frame
    /// [`Self::update_texture`] calls; `None` (the default) means "unknown /
    /// no surface", which pairs content with today's SDR-sRGB semantics.
    ///
    /// This is the other half of the content↔surface upload pairing table:
    /// the pairing function in this file is the table, this is the surface
    /// row it looks up. Never branch on it anywhere else (plan §5 design
    /// rule: concrete color spaces are named only in the pairing tables).
    active_surface_color_space: Option<wgpu::SurfaceColorSpace>,

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
    /// Creates a renderer for a egui UI.
    ///
    /// Render pipelines are created lazily, keyed by the format of the
    /// render-pass target, the first time [`Self::update_buffers`] sees a
    /// format. Pass the actual target format of each render pass to
    /// [`Self::render`] and [`Self::update_buffers`].
    pub fn new(device: &wgpu::Device, options: RendererOptions) -> Self {
        profiling::function_scope!();

        let shader = wgpu::ShaderModuleDescriptor {
            label: Some("egui"),
            // The module is the shared transfer functions (`egui_transfer.wgsl`)
            // concatenated with the entry points (`egui.wgsl`); the CPU↔GPU
            // round-trip test concatenates the same helpers with its own entry
            // points, so both exercise the same WGSL source. The allocation is
            // once per `Renderer::new`, never per frame.
            //
            // Two maintainer traps:
            // - WGSL error messages report line numbers against the
            //   CONCATENATED source. To find a reported line in `egui.wgsl`,
            //   subtract `egui_transfer.wgsl`'s line count plus the join
            //   newline.
            // - Nothing validates this concatenation at build time: `cargo
            //   check`, clippy and the wasm32 CI job never parse WGSL. The
            //   first validation of any edit to either file is this call, on
            //   a machine with a real wgpu adapter.
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(format!(
                "{}\n{}",
                include_str!("egui_transfer.wgsl"),
                include_str!("egui.wgsl")
            ))),
        };
        let module = {
            profiling::scope!("create_shader_module");
            device.create_shader_module(shader)
        };

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("egui_uniform_buffer"),
            contents: bytemuck::cast_slice(&[UniformBuffer {
                screen_size_in_points: [0.0, 0.0],
                dithering: u32::from(options.dithering),
                predictable_texture_filtering: u32::from(options.predictable_texture_filtering),
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
            upload_rejections_warned: HashSet::default(),
            options,
            callback_resources: CallbackResources::default(),
        }
    }

    /// Creates (if needed) the render pipeline for `target_format`.
    ///
    /// Exactly one pipeline is cached per target format; the cache is shared by
    /// all viewports, so viewports rendering at different formats coexist.
    ///
    /// The pipeline is compiled outside the lock: pipeline creation is expensive
    /// and must not stall a concurrent `render` reading the cache. A racing
    /// double compile is benign — the first pipeline inserted wins.
    fn ensure_pipeline(&self, device: &wgpu::Device, target_format: wgpu::TextureFormat) {
        if self.pipelines.lock().contains_key(&target_format) {
            return;
        }

        let pipeline = Arc::new(create_pipeline(
            device,
            &self.shader_module,
            &self.pipeline_layout,
            &self.options,
            target_format,
        ));
        self.pipelines
            .lock()
            .entry(target_format)
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
    /// Panics if [`Renderer::update_buffers`] has never been called with this `target_format`!
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
            pipelines.get(&target_format).cloned().expect(
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
    /// pass draws into; the pipeline for it is created here if this is its first
    /// use.
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
    /// The active surface pairs a linear-storage color space (scRGB); the
    /// gamma-encoded float contract has no row there yet — that pairing
    /// (with its linear conversion) is a later stage.
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
                "this surface's color space (scRGB-linear) stores linear values, but the float \
                 content contract is gamma-encoded; the linear pairing arrives with egui's \
                 scRGB stage. Skipping the upload (the placeholder renders)"
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
/// | `DisplayP3` | `DisplayP3` (± extended)     | native              |
/// | `DisplayP3` | sRGB / extended-sRGB         | loud reject         |
/// | either      | scRGB-linear                 | loud reject (linear pairing is a later stage) |
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
        // the chrome gets on a P3-referred surface (§4 row 2; §5).
        (
            epaint::Primaries::Srgb,
            Scs::Auto | Scs::Srgb | Scs::ExtendedSrgb | Scs::DisplayP3 | Scs::ExtendedDisplayP3,
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

        // scRGB-linear stores linear values; the gamma-encoded contract has
        // no row there until the linear pairing lands (§4's PR 4 row, kept).
        (_, Scs::ExtendedSrgbLinear) => Err(PairingRejection::LinearStorageNotPairedYet),

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
/// The per-format entry-point choice is the gamma-passthrough sniff it has
/// always been ([`wgpu::TextureFormat::is_srgb`]); deriving the encoding from
/// anything richer is future work (plan §3).
fn create_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    options: &RendererOptions,
    target_format: wgpu::TextureFormat,
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
            entry_point: Some(if target_format.is_srgb() {
                log::warn!("Detected a linear (sRGBA aware) framebuffer {target_format:?}. egui prefers Rgba8Unorm or Bgra8Unorm");
                "fs_main_linear_framebuffer"
            } else {
                "fs_main_gamma_framebuffer" // this is what we prefer
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

    /// The pipeline cache is keyed by target format: repeated renders at one
    /// format reuse one pipeline, a second format gets its own entry, and no
    /// device error surfaces along the way (plan §8, row 1).
    ///
    /// Adapter-requiring: this is egui-wgpu's first runtime test. Machines
    /// without any wgpu adapter skip with a message; CI and normal dev
    /// machines (hardware GPU or software rasterizer) always run it.
    #[test]
    fn pipeline_cache_is_keyed_by_target_format() {
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
                        "skipping pipeline_cache_is_keyed_by_target_format: no wgpu adapter: {err}"
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

        render_offscreen(
            &device,
            &queue,
            &mut renderer,
            wgpu::TextureFormat::Bgra8Unorm,
        );
        let first_bgra = renderer
            .pipelines
            .lock()
            .get(&wgpu::TextureFormat::Bgra8Unorm)
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
                        .get(&wgpu::TextureFormat::Bgra8Unorm)
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
                    .get(&wgpu::TextureFormat::Rgba8Unorm)
                    .expect("the Rgba8Unorm pipeline must be cached"),
            ),
            "distinct target formats must hold distinct pipelines"
        );
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

    /// Decode IEEE 754 binary16 bits to `f32`.
    ///
    /// Hand-rolled to keep the test dependency-free; sanity-checked by
    /// [`f16_bits_to_f32_known_values`] below.
    fn f16_bits_to_f32(h: u16) -> f32 {
        let sign = u32::from(h & 0x8000) << 16;
        let exp = u32::from((h >> 10) & 0x1f);
        let frac = u32::from(h & 0x03ff);
        let magnitude = match exp {
            0 if frac == 0 => 0, // ±0
            0 => {
                // Subnormal: normalize. `ilog2` is the index of the highest
                // set bit within the 10-bit fraction field.
                let msb = frac.ilog2();
                ((103 + msb) << 23) | ((frac << (23 - msb)) & 0x007f_ffff)
            }
            31 => 0x7f80_0000 | (frac << 13), // inf / NaN
            exp => ((exp + 112) << 23) | (frac << 13),
        };
        f32::from_bits(sign | magnitude)
    }

    #[test]
    fn f16_bits_to_f32_known_values() {
        assert_eq!(f16_bits_to_f32(0x3C00), 1.0);
        assert_eq!(f16_bits_to_f32(0xBC00), -1.0);
        assert_eq!(f16_bits_to_f32(0x0000), 0.0);
        assert_eq!(f16_bits_to_f32(0x0000).to_bits(), 0.0f32.to_bits());
        assert_eq!(f16_bits_to_f32(0x8000), 0.0);
        assert_eq!(f16_bits_to_f32(0x8000).to_bits(), (-0.0f32).to_bits());
        assert_eq!(f16_bits_to_f32(0x8400), -2.0f32.powi(-14)); // sign + min normal
        assert_eq!(f16_bits_to_f32(0xC400), -4.0);
        assert_eq!(f16_bits_to_f32(0x0400), 2.0f32.powi(-14)); // smallest normal
        assert_eq!(f16_bits_to_f32(0x0001), 2.0f32.powi(-24)); // smallest subnormal
        assert_eq!(f16_bits_to_f32(0x03FF), 1023.0 * 2.0f32.powi(-24));
        assert_eq!(f16_bits_to_f32(0x3555), 0.333_251_95);
        assert_eq!(f16_bits_to_f32(0x7C00), f32::INFINITY);
        assert!(f16_bits_to_f32(0x7E00).is_nan());
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
            .expect("GPU poll failed while waiting for the readback map");
        receiver
            .recv()
            .expect("readback sender hung up")
            .expect("failed to map the readback buffer");

        let data = slice
            .get_mapped_range()
            .expect("failed to map the readback buffer");
        let mut pixels = Vec::with_capacity((size * size) as usize);
        for row in 0..size {
            for col in 0..size {
                let offset = (row * bytes_per_row + col * 8) as usize; // 4 × f16
                let texel: [u16; 4] = bytemuck::cast_slice(&data[offset..offset + 8])
                    .try_into()
                    .expect("each f16 texel is exactly eight bytes");
                pixels.push([
                    f16_bits_to_f32(texel[0]),
                    f16_bits_to_f32(texel[1]),
                    f16_bits_to_f32(texel[2]),
                    f16_bits_to_f32(texel[3]),
                ]);
            }
        }
        pixels
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
                    // Srgb content: native on sRGB spaces and on DisplayP3
                    // (shifts like the chrome); rejected on the linear
                    // scRGB pairing and on non-grantable BT.2100 spaces.
                    (
                        epaint::Primaries::Srgb,
                        Scs::Auto
                        | Scs::Srgb
                        | Scs::ExtendedSrgb
                        | Scs::DisplayP3
                        | Scs::ExtendedDisplayP3,
                    ) => {
                        assert_eq!(result, Ok(()), "{primaries:?} on {space:?}");
                    }
                    (epaint::Primaries::Srgb, Scs::ExtendedSrgbLinear) => {
                        assert_eq!(
                            result,
                            Err(PairingRejection::LinearStorageNotPairedYet),
                            "{primaries:?} on {space:?}"
                        );
                    }
                    // DisplayP3 content: native on P3 (± extended), loud
                    // reject everywhere else (sRGB spaces gamut-clip; scRGB
                    // is the later linear pairing; BT.2100 not grantable).
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

        // The scRGB-linear row is kept rejected for BOTH tags (PR 4 pairing).
        renderer.set_active_surface_color_space(Some(wgpu::SurfaceColorSpace::ExtendedSrgbLinear));
        let p3_id_scrgb = epaint::TextureId::Managed(202);
        renderer.update_texture(&device, &queue, p3_id_scrgb, &p3_delta);
        assert!(renderer.texture(&p3_id_scrgb).is_none());
        let srgb_id_scrgb = epaint::TextureId::Managed(203);
        renderer.update_texture(&device, &queue, srgb_id_scrgb, &srgb_delta);
        assert!(renderer.texture(&srgb_id_scrgb).is_none());
        assert!(
            renderer.upload_rejections_warned.contains(&srgb_id_scrgb),
            "gamma-encoded content on the linear pairing must reject loudly"
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
}
